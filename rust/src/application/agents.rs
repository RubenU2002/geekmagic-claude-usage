//! Is Claude / Codex working right now, waiting for you, or done? Followed one session at a time, since you may be working
//! in several projects at once and each one's run starts, asks you something and finishes on its own.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::{config, Callback, Notify};
use crate::core::model::Data;
use crate::core::ports::AgentState;
use crate::core::registry::ProviderRegistry;
use crate::core::sessions::{duration_text, SessionState};
use crate::core::views::is_panel_key;
use crate::util::{epoch, Event, LockExt};

/// What is known of one session (see `track_session`): one per project you work in.
#[derive(Debug, Clone, Default)]
struct Track {
    working: bool,
    waiting: bool,
    waiting_for: Option<String>,
    started: Option<f64>,
    idle_polls: u32,
    wait_polls: u32,
    project: Option<String>,
}

/// A provider's agents, summed up: is any of them working / waiting?
#[derive(Debug, Clone, Default)]
pub struct AgentSummary {
    pub working: bool,
    pub waiting: bool,
    pub waiting_for: Option<String>,
    pub project: Option<String>,
    pub since: Option<f64>,
    pub sessions: usize,
}

/// Polls the providers' logs and turns what they say into per-session tracks.
///
/// The app hooks in through `on_change()` (an agent started or stopped: redraw) and `notify(title, message)`.
pub struct AgentMonitor {
    providers: Arc<ProviderRegistry>,
    notify: Notify,
    on_change: Callback,
    stop: Arc<Event>,
    events_enabled: Arc<dyn Fn() -> bool + Send + Sync>, // is the "tell me when an agent finishes or waits" switch on
    agent: Mutex<HashMap<String, AgentSummary>>,
    tracks: Mutex<HashMap<String, HashMap<String, Track>>>, // provider -> session id -> track
    /// An agent started or stopped: redraw from what's already read.
    dirty: AtomicBool,
    /// A webhook says something happened: look at the logs now instead of at the next poll.
    nudge: Event,
}

impl AgentMonitor {
    pub fn new(providers: Arc<ProviderRegistry>, notify: Notify, on_change: Callback, stop: Arc<Event>, events_enabled: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self {
            providers,
            notify,
            on_change,
            stop,
            events_enabled,
            agent: Mutex::new(HashMap::new()),
            tracks: Mutex::new(HashMap::new()),
            dirty: AtomicBool::new(false),
            nudge: Event::new(),
        }
    }

    /// Take (and clear) the "something changed" flag.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::SeqCst)
    }

    /// Look at the logs now (a webhook said something happened), and keep looking quickly for a while.
    pub fn nudge(&self) {
        self.nudge.set();
    }

    // --- what the screens ask -------------------------------------------------------------------------------

    pub fn working(&self, provider: Option<&str>) -> bool {
        provider.is_some_and(|p| self.agent.lk().get(p).is_some_and(|a| a.working))
    }

    pub fn waiting(&self, provider: Option<&str>) -> bool {
        provider.is_some_and(|p| self.agent.lk().get(p).is_some_and(|a| a.waiting))
    }

    pub fn summary_of(&self, provider: &str) -> AgentSummary {
        self.agent.lk().get(provider).cloned().unwrap_or_default()
    }

    /// What's about to be drawn, with each provider's "its agent is working right now" flag set from the latest look at
    /// its logs (the screen draws a tag and the mascot works). `key` is a provider's (`data` is its Usage, or an
    /// ErrorScreen), or one of the views of two (`data` is a list of Usage).
    pub fn flag(&self, key: &str, data: Data) -> Data {
        match data {
            Data::One(mut usage) if self.providers.contains(key) => {
                usage.working = self.working(Some(key));
                usage.waiting = self.waiting(Some(key));
                Data::One(usage)
            }
            Data::Panels(mut panels) if is_panel_key(key) => {
                for usage in &mut panels {
                    let provider = self.providers.key_of(&usage.title);
                    usage.working = self.working(provider.as_deref());
                    usage.waiting = self.waiting(provider.as_deref());
                }
                Data::Panels(panels)
            }
            other => other,
        }
    }

    // --- polling --------------------------------------------------------------------------------------------

    /// Thread: look at the agents' logs, even while paused or locked (that's when you want to hear that one finished).
    /// Every couple of seconds while an agent is busy (a run ends after a few quiet polls in a row), and rarely when none
    /// is: a webhook says when to look, so there is nothing to watch for.
    pub fn watch(&self) {
        while !self.stop.is_set() {
            self.poll();
            let busy = self.agent.lk().values().any(|a| a.working);
            self.nudge.wait(if busy { config::AGENT_POLL } else { config::AGENT_IDLE_POLL });
            self.nudge.clear();
        }
    }

    pub fn poll(&self) {
        for provider in self.providers.iter() {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider.agent_state(epoch()))) {
                Ok(state) => self.update(provider.key(), &state),
                Err(_) => log::warn!("could not read {}'s activity", provider.key()),
            }
        }
    }

    /// Follow a provider's agents, one session at a time. The screens only need the sum: is any of them working /
    /// waiting?
    pub fn update(&self, provider: &str, state: &AgentState) {
        let mut notes: Vec<(String, String)> = Vec::new();
        let mut changed = false;
        {
            let mut all = self.tracks.lk();
            let tracks = all.entry(provider.to_string()).or_default();
            let seen: Vec<&str> = state.sessions.iter().map(|s| if s.id.is_empty() { "-" } else { s.id.as_str() }).collect();
            for (s, sid) in state.sessions.iter().zip(&seen) {
                changed |= self.track_session(provider, sid, s, tracks, &mut notes);
            }
            let gone: Vec<String> = tracks.keys().filter(|sid| !seen.contains(&sid.as_str())).cloned().collect();
            for sid in gone {
                // its log went quiet for good: it can only be idle
                changed |= self.track_session(provider, &sid, &SessionState::default(), tracks, &mut notes);
            }
            tracks.retain(|_, t| t.working); // an idle session needs no memory: the debounce counters only matter while it works
            let busy: Vec<&Track> = tracks.values().filter(|t| t.working).collect();
            let waiting: Vec<&&Track> = busy.iter().filter(|t| t.waiting).collect();
            let lead = waiting.first().map(|t| **t).or(busy.first().copied());
            self.agent.lk().insert(
                provider.to_string(),
                AgentSummary {
                    working: !busy.is_empty(),
                    waiting: !waiting.is_empty(),
                    waiting_for: if waiting.is_empty() { None } else { lead.and_then(|t| t.waiting_for.clone()) },
                    project: if busy.is_empty() { state.project.clone() } else { lead.and_then(|t| t.project.clone()) },
                    since: busy.iter().filter_map(|t| t.started).reduce(f64::min),
                    sessions: busy.len(),
                },
            );
        }
        for (title, message) in notes {
            (self.notify)(&title, &message);
        }
        if changed {
            self.dirty.store(true, Ordering::SeqCst);
            (self.on_change)();
        }
    }

    /// One session: a run starting shows at once; a question is believed after AGENT_WAIT_POLLS polls in a row and a run
    /// ending after AGENT_IDLE_POLLS quiet ones, so the gaps between its steps don't flicker or notify early. Returns
    /// whether something visible changed. Notifications are appended to `notes` and sent by the caller.
    fn track_session(&self, provider: &str, sid: &str, s: &SessionState, tracks: &mut HashMap<String, Track>, notes: &mut Vec<(String, String)>) -> bool {
        let t = tracks.entry(sid.to_string()).or_default();
        if s.project.as_deref().is_some_and(|p| !p.is_empty()) {
            t.project = s.project.clone();
        }
        let mut changed = false;
        if s.working {
            t.idle_polls = 0;
            if !t.working {
                t.working = true;
                t.started = Some(s.since.unwrap_or_else(epoch));
                log::info!("{provider} started working ({})", t.project.as_deref().unwrap_or("None"));
                changed = true;
            }
            t.wait_polls = if s.waiting { t.wait_polls + 1 } else { 0 };
            let waiting_now = s.waiting && (t.waiting || t.wait_polls >= config::AGENT_WAIT_POLLS);
            if waiting_now != t.waiting {
                t.waiting = waiting_now;
                t.waiting_for = if waiting_now { s.waiting_for.clone() } else { None };
                log::info!(
                    "{provider} ({}) {}",
                    t.project.as_deref().unwrap_or("None"),
                    if waiting_now { format!("is waiting for you ({})", t.waiting_for.as_deref().unwrap_or("None")) } else { "is no longer waiting for you".into() }
                );
                if waiting_now {
                    if let Some(note) = self.waiting_note(provider, t) {
                        notes.push(note);
                    }
                }
                changed = true;
            }
        } else if t.working {
            t.idle_polls += 1;
            if t.idle_polls >= config::AGENT_IDLE_POLLS {
                let lasted = t.started.map_or(0.0, |started| epoch() - started);
                log::info!("{provider} ({}) finished (worked {})", t.project.as_deref().unwrap_or("None"), duration_text(lasted));
                if (self.events_enabled)() && lasted >= config::AGENT_MIN_RUN {
                    notes.push((self.title(provider, t.project.as_deref()), format!("Terminó (trabajó {})", duration_text(lasted))));
                }
                t.working = false;
                t.waiting = false;
                t.waiting_for = None;
                t.started = None;
                t.wait_polls = 0;
                changed = true;
            }
        }
        changed
    }

    // --- what it says ---------------------------------------------------------------------------------------

    /// "Claude · Acme App": the project is in the title, which is what you read first when you have several going.
    pub fn title(&self, provider: &str, project: Option<&str>) -> String {
        let name = self.providers.title_of(provider);
        match project.filter(|p| !p.is_empty()) {
            Some(p) => format!("{name} · {p}"),
            None => name,
        }
    }

    /// It stopped and needs you: say what for (a question, a plan to approve, a tool to allow); the title says where.
    fn waiting_note(&self, provider: &str, session: &Track) -> Option<(String, String)> {
        let reason = session.waiting_for.as_deref().unwrap_or("");
        let what = if reason == "question" {
            "Te hizo una pregunta".to_string()
        } else if reason == "plan" {
            "Espera que apruebes su plan".to_string()
        } else if let Some(rest) = reason.strip_prefix("approval") {
            format!("Espera tu aprobación{rest}") // "approval (Edit)" -> "...aprobación (Edit)"
        } else {
            "Espera tu respuesta".to_string()
        };
        (self.events_enabled)().then(|| (self.title(provider, session.project.as_deref()), what))
    }

    pub fn tooltip_notes(&self) -> Vec<String> {
        self.providers
            .titles()
            .into_iter()
            .filter(|(key, _)| self.working(Some(key)))
            .map(|(key, title)| if self.waiting(Some(&key)) { format!("{title} te espera") } else { format!("{title} trabajando") })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::testing::Said;
    use crate::application::usage::tests::{registry, Scripted};
    use crate::core::model::{Usage, Window};
    use std::sync::atomic::AtomicUsize;

    struct Rig {
        monitor: AgentMonitor,
        said: Arc<Said>,
        changes: Arc<AtomicUsize>,
        events: Arc<AtomicBool>,
    }

    fn rig() -> Rig {
        let said = Said::new();
        let changes = Arc::new(AtomicUsize::new(0));
        let events = Arc::new(AtomicBool::new(true));
        let (c, e) = (changes.clone(), events.clone());
        let reg = registry(vec![Scripted::new("claude", "Claude", vec![Ok((1.0, 1.0))]), Scripted::new("codex", "Codex", vec![Ok((1.0, 1.0))])]);
        let monitor = AgentMonitor::new(reg, said.notify(), Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }), Arc::new(Event::new()), Arc::new(move || e.load(Ordering::SeqCst)));
        Rig { monitor, said, changes, events }
    }

    fn session(id: &str, working: bool, waiting: Option<&str>, project: &str) -> SessionState {
        SessionState {
            id: id.into(),
            working,
            waiting: waiting.is_some(),
            waiting_for: waiting.map(String::from),
            since: working.then(|| epoch() - 300.0),
            last: epoch(),
            project: Some(project.into()),
        }
    }

    fn state(sessions: Vec<SessionState>) -> AgentState {
        AgentState { sessions, ..Default::default() }
    }

    #[test]
    fn a_run_shows_at_once_and_ends_only_after_quiet_polls() {
        let r = rig();
        r.monitor.update("claude", &state(vec![session("s1", true, None, "Acme")]));
        assert!(r.monitor.working(Some("claude")) && r.monitor.take_dirty());
        assert_eq!(r.monitor.summary_of("claude").project.as_deref(), Some("Acme"));
        r.monitor.update("claude", &state(vec![session("s1", false, None, "Acme")]));
        assert!(r.monitor.working(Some("claude")), "one quiet poll is the gap between steps, not the end");
        assert!(r.said.all().is_empty());
        r.monitor.update("claude", &state(vec![session("s1", false, None, "Acme")]));
        assert!(!r.monitor.working(Some("claude")));
        assert_eq!(r.said.all(), [("Claude · Acme".to_string(), "Terminó (trabajó 5 min)".to_string())]);
    }

    #[test]
    fn a_short_run_ends_quietly_and_so_does_a_muted_one() {
        let r = rig();
        let mut quick = session("s1", true, None, "Acme");
        quick.since = Some(epoch() - 5.0);
        r.monitor.update("claude", &state(vec![quick]));
        for _ in 0..2 {
            r.monitor.update("claude", &state(vec![]));
        }
        assert!(r.said.all().is_empty(), "under a minute: no notification");
        r.events.store(false, Ordering::SeqCst);
        r.monitor.update("claude", &state(vec![session("s2", true, None, "Acme")]));
        for _ in 0..2 {
            r.monitor.update("claude", &state(vec![]));
        }
        assert!(r.said.all().is_empty(), "muted");
    }

    #[test]
    fn a_question_is_believed_after_two_polls_and_tells_what_for() {
        let r = rig();
        let asking = || state(vec![session("s1", true, Some("question"), "Acme")]);
        r.monitor.update("claude", &asking());
        assert!(!r.monitor.waiting(Some("claude")) && r.said.all().is_empty());
        r.monitor.update("claude", &asking());
        assert!(r.monitor.waiting(Some("claude")));
        assert_eq!(r.said.all(), [("Claude · Acme".to_string(), "Te hizo una pregunta".to_string())]);
        r.monitor.update("claude", &asking());
        assert_eq!(r.said.all().len(), 1, "said once");
        assert_eq!(r.monitor.tooltip_notes(), ["Claude te espera"]);
        r.monitor.update("claude", &state(vec![session("s1", true, None, "Acme")]));
        assert!(!r.monitor.waiting(Some("claude")) && r.monitor.working(Some("claude")));
        assert_eq!(r.monitor.tooltip_notes(), ["Claude trabajando"]);
    }

    #[test]
    fn what_the_agent_waits_for_is_worded_for_the_user() {
        for (reason, said) in [("plan", "Espera que apruebes su plan"), ("approval (Edit)", "Espera tu aprobación (Edit)"), ("approval", "Espera tu aprobación"), ("other", "Espera tu respuesta")] {
            let r = rig();
            for _ in 0..2 {
                r.monitor.update("claude", &state(vec![session("s1", true, Some(reason), "P")]));
            }
            assert_eq!(r.said.all()[0].1, said, "{reason}");
        }
    }

    #[test]
    fn several_sessions_are_followed_apart_and_a_vanished_one_counts_as_idle() {
        let r = rig();
        r.monitor.update("claude", &state(vec![session("s1", true, None, "Alpha"), session("s2", true, None, "Beta")]));
        assert_eq!(r.monitor.summary_of("claude").sessions, 2);
        for _ in 0..2 {
            r.monitor.update("claude", &state(vec![session("s2", true, None, "Beta")])); // s1's log went quiet for good
        }
        let summary = r.monitor.summary_of("claude");
        assert_eq!((summary.sessions, summary.project.as_deref()), (1, Some("Beta")));
        assert!(r.said.titles().contains(&"Claude · Alpha".to_string()), "Alpha's end was announced under its own name");
    }

    #[test]
    fn providers_are_independent_and_flags_land_on_the_right_panel() {
        let r = rig();
        r.monitor.update("codex", &state(vec![session("c1", true, None, "Gamma")]));
        assert!(r.monitor.working(Some("codex")) && !r.monitor.working(Some("claude")) && !r.monitor.working(None));
        let usage = |t: &str| Usage::new(t, Window::default(), Window::default(), chrono::Local::now());
        let Data::Panels(panels) = r.monitor.flag("split", Data::Panels(vec![usage("Claude"), usage("Codex")])) else { panic!() };
        assert_eq!((panels[0].working, panels[1].working), (false, true));
        let Data::One(one) = r.monitor.flag("codex", Data::One(usage("Codex"))) else { panic!() };
        assert!(one.working);
        let err = Data::Error(crate::core::model::ErrorScreen { title: "Codex".into(), message: "m".into(), now: chrono::Local::now(), signin: false });
        assert!(matches!(r.monitor.flag("codex", err), Data::Error(_)), "an error screen is left alone");
        assert_eq!(r.changes.load(Ordering::SeqCst), 1);
    }
}
