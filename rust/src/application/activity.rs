//! The providers' activity, counted from their local logs (up to a month of them) for the stats views.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::{config, Callback};
use crate::core::model::Activity;
use crate::core::registry::ProviderRegistry;
use crate::util::{epoch, mono, LockExt};

/// Counts now and then, not every cycle, and in a thread, since the first count takes seconds. Until it's done the
/// screens say "Counting...". `on_done()` wakes the loop to redraw the screen that was saying so.
pub struct ActivityCounter {
    providers: Arc<ProviderRegistry>,
    on_done: Callback,
    background: bool,
    /// provider -> its activity from its local logs; `NoLogs` = it has none, missing = not counted yet
    stats: Mutex<HashMap<String, Activity>>,
    counted_at: Mutex<f64>,
    counting: AtomicBool, // the counts are being computed in a thread
}

impl ActivityCounter {
    pub fn new(providers: Arc<ProviderRegistry>, on_done: Callback, background: bool) -> Arc<Self> {
        Arc::new(Self {
            providers,
            on_done,
            background,
            stats: Mutex::new(HashMap::new()),
            counted_at: Mutex::new(0.0),
            counting: AtomicBool::new(false),
        })
    }

    /// What a provider's stats views show: its numbers, "no logs", or "counting" while they aren't there yet.
    pub fn activity_of(&self, provider: &str) -> Activity {
        self.stats.lk().get(provider).cloned().unwrap_or(Activity::Counting)
    }

    pub fn refresh(self: &Arc<Self>) {
        let since = mono() - *self.counted_at.lk();
        let complete = self.stats.lk().len() == self.providers.len();
        if self.counting.load(Ordering::SeqCst) || since < if complete { config::LOCAL_STATS_EVERY } else { config::LOCAL_STATS_RETRY } {
            return;
        }
        *self.counted_at.lk() = mono();
        if self.background {
            self.counting.store(true, Ordering::SeqCst);
            let me = self.clone();
            crate::util::in_background(move || me.count());
        } else {
            self.count();
        }
    }

    fn count(&self) {
        for provider in self.providers.iter() {
            let counted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider.local_stats(epoch())));
            match counted {
                Ok(stats) => {
                    let activity = stats.map_or(Activity::NoLogs, |s| Activity::Stats(Arc::new(s)));
                    self.stats.lk().insert(provider.key().to_string(), activity);
                }
                Err(_) => log::warn!("could not count {}'s local activity", provider.key()),
            }
        }
        self.counting.store(false, Ordering::SeqCst);
        if self.background {
            (self.on_done)(); // redraw the screen that was saying "Counting..."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::usage::tests::registry;
    use crate::core::activity::Summary;
    use crate::core::errors::UsageError;
    use crate::core::model::Usage;
    use crate::core::ports::{AgentState, Provider};
    use std::sync::atomic::AtomicUsize;

    struct Counted {
        key: &'static str,
        logs: bool,
        calls: AtomicUsize,
    }

    impl Provider for Counted {
        fn key(&self) -> &str {
            self.key
        }
        fn title(&self) -> &str {
            self.key
        }
        fn login_args(&self) -> &[&str] {
            &[]
        }
        fn install_hint(&self) -> &str {
            ""
        }
        fn fetch(&self) -> Result<Usage, UsageError> {
            Err(UsageError::new("unused"))
        }
        fn find_cli(&self) -> Option<String> {
            None
        }
        fn local_stats(&self, _: f64) -> Option<Summary> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.logs.then(Summary::default)
        }
        fn agent_state(&self, _: f64) -> AgentState {
            AgentState::default()
        }
    }

    #[test]
    fn counts_once_then_waits_and_tells_logs_from_no_logs() {
        let with = Arc::new(Counted { key: "a", logs: true, calls: AtomicUsize::new(0) });
        let without = Arc::new(Counted { key: "b", logs: false, calls: AtomicUsize::new(0) });
        let done = Arc::new(AtomicUsize::new(0));
        let d = done.clone();
        let counter = ActivityCounter::new(registry(vec![with.clone(), without.clone()]), Arc::new(move || {
            d.fetch_add(1, Ordering::SeqCst);
        }), false);
        assert!(matches!(counter.activity_of("a"), Activity::Counting), "nothing counted yet");
        counter.refresh();
        assert!(matches!(counter.activity_of("a"), Activity::Stats(_)));
        assert!(matches!(counter.activity_of("b"), Activity::NoLogs));
        counter.refresh();
        assert_eq!(with.calls.load(Ordering::SeqCst), 1, "not again until the interval has passed");
        assert_eq!(done.load(Ordering::SeqCst), 0, "the foreground mode has nobody to wake");
    }

    #[test]
    fn in_the_background_it_wakes_the_loop_when_done() {
        let p = Arc::new(Counted { key: "a", logs: true, calls: AtomicUsize::new(0) });
        let woke = Arc::new(crate::util::Event::new());
        let w = woke.clone();
        let counter = ActivityCounter::new(registry(vec![p]), Arc::new(move || w.set()), true);
        counter.refresh();
        assert!(woke.wait(5.0));
        assert!(matches!(counter.activity_of("a"), Activity::Stats(_)));
    }
}
