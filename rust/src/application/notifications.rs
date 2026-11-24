//! Desktop notifications, and the two switches that turn them off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::persistence::Part;
use crate::core::ports::Notifier;

pub struct Notifications {
    notifier: Arc<dyn Notifier>,
    enabled: AtomicBool,      // every notification
    agent_events: AtomicBool, // the ones about an agent finishing or waiting for you
}

impl Notifications {
    pub fn new(notifier: Arc<dyn Notifier>) -> Self {
        Self { notifier, enabled: AtomicBool::new(true), agent_events: AtomicBool::new(true) }
    }

    pub fn send(&self, title: &str, message: &str) {
        log::info!("notification: {title} - {message}");
        if !self.enabled() {
            return;
        }
        // (it must never break the update loop)
        if let Err(e) = self.notifier.send(title, message) {
            log::warn!("could not show the notification: {e}");
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn agent_events(&self) -> bool {
        self.agent_events.load(Ordering::SeqCst)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
    }

    pub fn set_agent_events(&self, on: bool) {
        self.agent_events.store(on, Ordering::SeqCst);
    }
}

impl Part for Notifications {
    fn name(&self) -> &'static str {
        "notifications"
    }

    fn restore(&self, saved: &Value) {
        self.set_enabled(saved.get("notify").and_then(Value::as_bool).unwrap_or(true));
        self.set_agent_events(saved.get("notify_done").and_then(Value::as_bool).unwrap_or(true));
    }

    fn snapshot(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("notify".into(), json!(self.enabled()));
        m.insert("notify_done".into(), json!(self.agent_events()));
        m
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::util::LockExt;
    use std::sync::Mutex;

    /// A notifier that remembers what it was asked to show.
    #[derive(Default)]
    pub struct Recorder {
        pub sent: Mutex<Vec<(String, String)>>,
        pub fail: AtomicBool,
    }

    impl Notifier for Recorder {
        fn send(&self, title: &str, message: &str) -> Result<(), String> {
            if self.fail.load(Ordering::SeqCst) {
                return Err("no way".into());
            }
            self.sent.lk().push((title.into(), message.into()));
            Ok(())
        }
    }

    #[test]
    fn sends_unless_switched_off_and_never_fails_the_caller() {
        let r = Arc::new(Recorder::default());
        let n = Notifications::new(r.clone());
        n.send("Claude", "hi");
        n.set_enabled(false);
        n.send("Claude", "muted");
        n.set_enabled(true);
        r.fail.store(true, Ordering::SeqCst);
        n.send("Claude", "lost"); // (a failing notifier is logged, not raised)
        assert_eq!(r.sent.lk().len(), 1);
    }

    #[test]
    fn the_switches_are_remembered() {
        let n = Notifications::new(Arc::new(Recorder::default()));
        n.restore(&json!({"notify": false, "notify_done": false}));
        assert_eq!((n.enabled(), n.agent_events()), (false, false));
        assert_eq!(Value::Object(n.snapshot()), json!({"notify": false, "notify_done": false}));
        n.restore(&json!({}));
        assert_eq!((n.enabled(), n.agent_events()), (true, true));
    }
}
