//! Reading each provider's usage, and what is made of it: the recent pace, the alerts, which readings have gone stale.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Map, Value};

use super::persistence::Part;
use super::{config, Callback, Notify};
use crate::core::alerts::{self, Event};
use crate::core::model::{Usage, WindowKind};
use crate::core::pace;
use crate::core::registry::ProviderRegistry;
use crate::util::{epoch, mono, LockExt};

/// The recent readings of each usage window, from which the current pace is measured.
#[derive(Default)]
pub struct UsageHistory {
    points: Mutex<HashMap<String, Vec<[f64; 3]>>>, // "provider/window" -> [[epoch, pct, reset epoch], ...]
}

impl UsageHistory {
    /// Keep the readings of each window (when the percentage changed, or every few minutes) so the recent pace can be
    /// measured. Readings of an earlier window are dropped as soon as it resets.
    pub fn record(&self, provider: &str, usage: &Usage) {
        let now = epoch();
        let mut all = self.points.lk();
        for kind in WindowKind::ALL {
            let w = usage.window(kind);
            let (Some(pct), Some(reset)) = (w.pct, w.reset) else { continue };
            let reset = reset.timestamp() as f64;
            let points = all.entry(format!("{provider}/{}", kind.key())).or_default();
            points.retain(|p| (p[2] - reset).abs() < 120.0);
            if points.last().is_none_or(|last| last[1] != pct || now - last[0] >= config::history_every(kind.key())) {
                points.push([now, pct, reset]);
            }
            let extra = points.len().saturating_sub(config::HISTORY_MAX);
            points.drain(..extra);
        }
    }

    pub fn rates(&self, provider: &str, usage: &Usage) -> [Option<f64>; 2] {
        let all = self.points.lk();
        WindowKind::ALL.map(|kind| {
            let reset = usage.window(kind).reset.map(|r| r.timestamp() as f64);
            let points = all.get(&format!("{provider}/{}", kind.key())).map_or(&[][..], Vec::as_slice);
            pace::recent_rate(points, reset, epoch(), pace::lookback_min(kind))
        })
    }
}

impl Part for UsageHistory {
    fn name(&self) -> &'static str {
        "history"
    }

    fn restore(&self, saved: &Value) {
        let mut out = HashMap::new();
        for (key, value) in saved.get("history").and_then(Value::as_object).into_iter().flatten() {
            let Some(list) = value.as_array() else { continue };
            let mut points: Vec<[f64; 3]> = list
                .iter()
                .filter_map(|p| match p.as_array()?.as_slice() {
                    [a, b, c] => Some([a.as_f64()?, b.as_f64()?, c.as_f64()?]),
                    _ => None,
                })
                .collect();
            let extra = points.len().saturating_sub(config::HISTORY_MAX);
            points.drain(..extra);
            out.insert(key.clone(), points);
        }
        *self.points.lk() = out;
    }

    fn snapshot(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("history".into(), json!(*self.points.lk()));
        m
    }
}

/// Which usage alerts already fired, so each fires once (also across restarts).
pub struct AlertTracker {
    providers: Arc<ProviderRegistry>,
    notify: Notify,
    save: Callback,
    state: Mutex<HashMap<String, (usize, Option<f64>)>>, // "provider/window" -> (level 0..2, last reading)
}

impl AlertTracker {
    pub fn new(providers: Arc<ProviderRegistry>, notify: Notify, save: Callback) -> Self {
        Self { providers, notify, save, state: Mutex::new(HashMap::new()) }
    }

    /// Notify when a window newly reaches 80 % / 95 %, and when it resets after that.
    pub fn check(&self, provider: &str, usage: &Usage) {
        let title = self.providers.title_of(provider);
        let mut messages = Vec::new();
        {
            let mut state = self.state.lk();
            for kind in WindowKind::ALL {
                let Some(pct) = usage.window(kind).pct else { continue };
                let key = format!("{provider}/{}", kind.key());
                let (prev_level, prev_pct) = state.get(&key).copied().unwrap_or((0, None));
                let (level, event) = alerts::evaluate(prev_level, prev_pct, Some(pct));
                state.insert(key, (level, Some(pct)));
                match event {
                    Some(Event::Rise(threshold)) => {
                        messages.push(format!("La {} llegó al {} % (umbral {} %)", kind.label(), crate::util::pyround(pct), threshold));
                    }
                    Some(Event::Reset) => messages.push(format!("La {} se reinició ({} % usado)", kind.label(), crate::util::pyround(pct))),
                    None => {}
                }
            }
        }
        for message in messages {
            (self.notify)(&title, &message);
        }
        (self.save)();
    }
}

impl Part for AlertTracker {
    fn name(&self) -> &'static str {
        "alerts"
    }

    fn restore(&self, saved: &Value) {
        let mut out = HashMap::new();
        for (key, value) in saved.get("alerts").and_then(Value::as_object).into_iter().flatten() {
            if let Some(entry) = value.as_object() {
                let level = entry.get("level").and_then(Value::as_u64).unwrap_or(0).min(alerts::NOTIFY_PCTS.len() as u64) as usize;
                out.insert(key.clone(), (level, entry.get("pct").and_then(Value::as_f64)));
            }
        }
        *self.state.lk() = out;
    }

    fn snapshot(&self) -> Map<String, Value> {
        let alerts: Map<String, Value> =
            self.state.lk().iter().map(|(k, (level, pct))| (k.clone(), json!({"level": level, "pct": pct}))).collect();
        let mut m = Map::new();
        m.insert("alerts".into(), Value::Object(alerts));
        m
    }
}

#[derive(Default)]
struct Inner {
    last_fetch: HashMap<String, f64>,
    failures: HashMap<String, u32>, // provider -> consecutive failed reads
    retry_at: HashMap<String, f64>, // provider -> not before this (monotonic) once it keeps failing
    last_error: HashMap<String, String>,
    last_good: HashMap<String, (Usage, f64)>, // provider -> (last usage that was read OK, when)
    stale: HashSet<String>, // providers whose image on the device is the dimmed "no fresh data" one
    errors: HashMap<String, String>, // provider -> why its last read failed (gone once a read works)
    needs_login: HashSet<String>, // providers whose last read failed because you're signed out
}

type OnRead = Box<dyn Fn(&str) + Send + Sync>;
type OnFailure = Box<dyn Fn(&str, &str) + Send + Sync>;

/// Reads the providers and keeps the latest good reading of each, with why the last read failed if it did.
///
/// The app hooks in through `on_read(provider)` (after a good read) and `on_failure(provider, error)` (after a bad one).
pub struct UsageService {
    providers: Arc<ProviderRegistry>,
    history: Arc<UsageHistory>,
    alert_tracker: Arc<AlertTracker>,
    inner: Mutex<Inner>,
    on_read: OnceLock<OnRead>,
    on_failure: OnceLock<OnFailure>,
}

impl UsageService {
    pub fn new(providers: Arc<ProviderRegistry>, history: Arc<UsageHistory>, alert_tracker: Arc<AlertTracker>) -> Self {
        Self { providers, history, alert_tracker, inner: Mutex::new(Inner::default()), on_read: OnceLock::new(), on_failure: OnceLock::new() }
    }

    pub fn set_on_read(&self, f: impl Fn(&str) + Send + Sync + 'static) {
        let _ = self.on_read.set(Box::new(f));
    }

    pub fn set_on_failure(&self, f: impl Fn(&str, &str) + Send + Sync + 'static) {
        let _ = self.on_failure.set(Box::new(f));
    }

    fn failed(&self, provider: &str, error: &str) {
        if let Some(f) = self.on_failure.get() {
            f(provider, error);
        }
    }

    /// Query the provider. Every successful read also feeds the alerts; a failing one may mark the data stale. A provider
    /// that keeps failing (no internet, a VPN...) is left alone for a while instead of making every cycle wait for its
    /// timeout: see `reset_backoff`.
    pub fn fetch(&self, provider: &str) -> Option<Usage> {
        let now = mono();
        {
            let mut i = self.inner.lk();
            if now < i.retry_at.get(provider).copied().unwrap_or(0.0) {
                let error = i.last_error.get(provider).cloned().unwrap_or_default();
                drop(i);
                self.failed(provider, &error); // (still failing: it keeps the stale dimming going)
                return None;
            }
            i.last_fetch.insert(provider.to_string(), now);
        }
        let reading = match self.providers.get(provider) {
            Some(p) => catch_unwind(AssertUnwindSafe(|| p.fetch())).unwrap_or_else(|_| Err(crate::core::errors::UsageError::new("the reader crashed"))),
            None => Err(crate::core::errors::UsageError::new(format!("no provider called {provider}"))),
        };
        let mut usage = match reading {
            Ok(usage) => usage,
            Err(e) => {
                // an expected problem (not signed in...): its message says it all
                log::warn!("fetch failed ({provider}): {e}");
                {
                    let mut i = self.inner.lk();
                    if e.sign_in_needed {
                        i.needs_login.insert(provider.to_string());
                    }
                    i.errors.insert(provider.to_string(), e.message.clone());
                }
                self.back_off(provider, &e.message);
                self.failed(provider, &e.message);
                return None;
            }
        };
        self.history.record(provider, &usage);
        let rates = self.history.rates(provider, &usage);
        pace::annotate(&mut usage, rates); // the pace that goes under each bar
        {
            let mut i = self.inner.lk();
            i.last_good.insert(provider.to_string(), (usage.clone(), mono()));
            i.stale.remove(provider);
            i.errors.remove(provider);
            i.needs_login.remove(provider);
            i.failures.remove(provider);
            i.retry_at.remove(provider);
            i.last_error.remove(provider);
        }
        if let Some(f) = self.on_read.get() {
            f(provider);
        }
        self.alert_tracker.check(provider, &usage); // (which also saves)
        Some(usage)
    }

    fn back_off(&self, provider: &str, error: &str) {
        let mut i = self.inner.lk();
        i.last_error.insert(provider.to_string(), error.to_string());
        let count = {
            let c = i.failures.entry(provider.to_string()).or_insert(0);
            *c += 1;
            *c
        };
        if count >= config::BACKOFF_FROM {
            let wait = (config::BACKOFF_BASE * 2f64.powi((count - config::BACKOFF_FROM) as i32)).min(config::BACKOFF_MAX);
            i.retry_at.insert(provider.to_string(), mono() + wait);
            log::info!("{provider} failed {count} times in a row; not reading it again for {wait:.0} s");
        }
    }

    /// Try every provider at the next cycle (you asked for it: a click, "Actualizar ahora", a webhook).
    pub fn reset_backoff(&self) {
        self.inner.lk().retry_at.clear();
    }

    /// Forget when a provider was last read, so the next cycle that looks at it reads it again.
    pub fn forget_fetch(&self, provider: &str) {
        self.inner.lk().last_fetch.remove(provider);
    }

    pub fn last_fetch(&self, provider: &str) -> f64 {
        self.inner.lk().last_fetch.get(provider).copied().unwrap_or(0.0)
    }

    pub fn recently_read(&self, provider: &str, within: f64) -> bool {
        self.inner.lk().last_good.get(provider).is_some_and(|(_, at)| mono() - at < within)
    }

    pub fn last_good(&self, provider: &str) -> Option<(Usage, f64)> {
        self.inner.lk().last_good.get(provider).cloned()
    }

    pub fn has_good(&self, provider: &str) -> bool {
        self.inner.lk().last_good.contains_key(provider)
    }

    pub fn any_good(&self) -> bool {
        !self.inner.lk().last_good.is_empty()
    }

    /// The last good reading, if it has been too long since it was refreshed and it isn't already shown as stale.
    pub fn stale_candidate(&self, provider: &str) -> Option<Usage> {
        let i = self.inner.lk();
        let (usage, read_at) = i.last_good.get(provider)?;
        if i.stale.contains(provider) || mono() - read_at < config::STALE_SECONDS {
            return None;
        }
        Some(usage.clone())
    }

    pub fn mark_stale(&self, provider: &str) {
        self.inner.lk().stale.insert(provider.to_string());
    }

    pub fn error_of(&self, provider: &str) -> Option<String> {
        self.inner.lk().errors.get(provider).cloned()
    }

    pub fn set_error(&self, provider: &str, message: &str) {
        self.inner.lk().errors.insert(provider.to_string(), message.to_string());
    }

    pub fn has_error(&self, provider: &str) -> bool {
        self.inner.lk().errors.contains_key(provider)
    }

    /// Did its last read fail because you're signed out? (Only a read can tell: each CLI says so in its own way.)
    pub fn signed_out(&self, provider: &str) -> bool {
        self.inner.lk().needs_login.contains(provider)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::core::errors::UsageError;
    use crate::core::model::Window;
    use crate::core::ports::{AgentState, Provider};
    use chrono::{Duration, Local, Utc};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A provider that answers from a script: each fetch pops the next outcome (the last one repeats).
    pub struct Scripted {
        pub key: &'static str,
        pub title: &'static str,
        pub outcomes: Mutex<Vec<Result<(f64, f64), UsageError>>>,
        pub calls: AtomicUsize,
    }

    impl Scripted {
        pub fn new(key: &'static str, title: &'static str, outcomes: Vec<Result<(f64, f64), UsageError>>) -> Arc<Self> {
            Arc::new(Self { key, title, outcomes: Mutex::new(outcomes), calls: AtomicUsize::new(0) })
        }
    }

    impl Provider for Scripted {
        fn key(&self) -> &str {
            self.key
        }
        fn title(&self) -> &str {
            self.title
        }
        fn login_args(&self) -> &[&str] {
            &["login"]
        }
        fn install_hint(&self) -> &str {
            "install it"
        }
        fn fetch(&self) -> Result<Usage, UsageError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut q = self.outcomes.lk();
            let next = if q.len() > 1 { q.remove(0) } else { q[0].clone() };
            next.map(|(session, week)| {
                let now = Local::now();
                Usage::new(
                    self.title,
                    Window { pct: Some(session), reset: Some(now.with_timezone(&Utc) + Duration::minutes(120)), minutes: None },
                    Window { pct: Some(week), reset: Some(now.with_timezone(&Utc) + Duration::hours(72)), minutes: None },
                    now,
                )
            })
        }
        fn find_cli(&self) -> Option<String> {
            Some(format!("/bin/{}", self.key))
        }
        fn local_stats(&self, _: f64) -> Option<crate::core::activity::Summary> {
            None
        }
        fn agent_state(&self, _: f64) -> AgentState {
            AgentState::default()
        }
    }

    pub fn registry(providers: Vec<Arc<dyn Provider>>) -> Arc<ProviderRegistry> {
        Arc::new(ProviderRegistry::new(providers))
    }

    fn service(provider: Arc<Scripted>) -> (UsageService, Arc<Mutex<Vec<(String, String)>>>) {
        let notes = Arc::new(Mutex::new(Vec::new()));
        let sink = notes.clone();
        let reg = registry(vec![provider]);
        let alerts = Arc::new(AlertTracker::new(reg.clone(), Arc::new(move |t, m| sink.lk().push((t.into(), m.into()))), Arc::new(|| {})));
        (UsageService::new(reg, Arc::new(UsageHistory::default()), alerts), notes)
    }

    #[test]
    fn a_good_read_is_kept_annotated_and_clears_old_trouble() {
        let p = Scripted::new("claude", "Claude", vec![Err(UsageError::new("boom")), Ok((40.0, 10.0))]);
        let (svc, _) = service(p);
        assert!(svc.fetch("claude").is_none());
        assert_eq!(svc.error_of("claude").as_deref(), Some("boom"));
        let usage = svc.fetch("claude").unwrap();
        assert_eq!(usage.current.pct, Some(40.0));
        assert!(svc.error_of("claude").is_none() && svc.has_good("claude") && !svc.signed_out("claude"));
        assert!(svc.recently_read("claude", 30.0));
    }

    #[test]
    fn signed_out_is_remembered_until_a_read_works() {
        let p = Scripted::new("claude", "Claude", vec![Err(UsageError::sign_in("log in")), Ok((1.0, 1.0))]);
        let (svc, _) = service(p);
        svc.fetch("claude");
        assert!(svc.signed_out("claude"));
        svc.fetch("claude");
        assert!(!svc.signed_out("claude"));
    }

    #[test]
    fn a_provider_that_keeps_failing_is_left_alone_until_asked() {
        let p = Scripted::new("claude", "Claude", vec![Err(UsageError::new("no internet"))]);
        let (svc, _) = service(p.clone());
        let failures = Arc::new(Mutex::new(Vec::new()));
        let sink = failures.clone();
        svc.set_on_failure(move |p, e| sink.lk().push(format!("{p}: {e}")));
        for _ in 0..config::BACKOFF_FROM {
            svc.fetch("claude");
        }
        assert_eq!(p.calls.load(Ordering::SeqCst), config::BACKOFF_FROM as usize);
        assert!(svc.fetch("claude").is_none());
        assert_eq!(p.calls.load(Ordering::SeqCst), config::BACKOFF_FROM as usize, "not asked while backing off");
        assert_eq!(failures.lk().len(), config::BACKOFF_FROM as usize + 1, "but the failure is still reported (it keeps the dimming going)");
        svc.reset_backoff();
        svc.fetch("claude");
        assert_eq!(p.calls.load(Ordering::SeqCst), config::BACKOFF_FROM as usize + 1, "a click tries at once");
    }

    #[test]
    fn alerts_fire_once_per_threshold_and_on_reset() {
        let p = Scripted::new("claude", "Claude", vec![Ok((50.0, 1.0)), Ok((82.0, 1.0)), Ok((85.0, 1.0)), Ok((96.0, 1.0)), Ok((3.0, 1.0))]);
        let (svc, notes) = service(p);
        for _ in 0..5 {
            svc.fetch("claude");
        }
        let notes: Vec<String> = notes.lk().iter().map(|(t, m)| format!("{t}: {m}")).collect();
        assert_eq!(
            notes,
            ["Claude: La sesión llegó al 82 % (umbral 80 %)", "Claude: La sesión llegó al 96 % (umbral 95 %)", "Claude: La sesión se reinició (3 % usado)"]
        );
    }

    #[test]
    fn stale_candidates_need_an_old_reading_and_are_marked_once() {
        let p = Scripted::new("claude", "Claude", vec![Ok((10.0, 10.0))]);
        let (svc, _) = service(p);
        assert!(svc.stale_candidate("claude").is_none(), "never read");
        svc.fetch("claude");
        assert!(svc.stale_candidate("claude").is_none(), "just read");
        svc.inner.lk().last_good.get_mut("claude").unwrap().1 = mono() - config::STALE_SECONDS - 1.0;
        assert!(svc.stale_candidate("claude").is_some());
        svc.mark_stale("claude");
        assert!(svc.stale_candidate("claude").is_none(), "already shown as stale");
    }

    #[test]
    fn history_keeps_changes_drops_old_windows_and_survives_a_restart() {
        let h = UsageHistory::default();
        let mut u = Usage::new("Claude", Window::default(), Window::default(), Local::now());
        let reset = Utc::now() + Duration::minutes(100);
        u.current = Window { pct: Some(10.0), reset: Some(reset), minutes: None };
        h.record("claude", &u);
        h.record("claude", &u); // same % again, too soon: not added
        u.current.pct = Some(12.0);
        h.record("claude", &u);
        assert_eq!(h.snapshot()["history"]["claude/current"].as_array().unwrap().len(), 2);
        u.current.reset = Some(reset + Duration::hours(5)); // the window reset
        h.record("claude", &u);
        assert_eq!(h.snapshot()["history"]["claude/current"].as_array().unwrap().len(), 1);
        let again = UsageHistory::default();
        again.restore(&Value::Object(h.snapshot()));
        assert_eq!(again.snapshot()["history"], h.snapshot()["history"]);
        again.restore(&json!({"history": {"x/current": [[1, 2], [1, 2, 3], "bad"], "y": 5}}));
        assert_eq!(again.snapshot()["history"], json!({"x/current": [[1.0, 2.0, 3.0]]}));
    }

    #[test]
    fn alerts_are_remembered_across_restarts() {
        let reg = registry(vec![Scripted::new("claude", "Claude", vec![Ok((1.0, 1.0))])]);
        let t = AlertTracker::new(reg, Arc::new(|_, _| {}), Arc::new(|| {}));
        t.restore(&json!({"alerts": {"claude/current": {"level": 2, "pct": 96.0}, "junk": 5}}));
        assert_eq!(t.snapshot()["alerts"], json!({"claude/current": {"level": 2, "pct": 96.0}}));
    }
}
