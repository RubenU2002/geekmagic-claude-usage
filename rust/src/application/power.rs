//! Pausing: by hand, and by itself while the computer is locked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::persistence::Part;
use super::{config, Callback};
use crate::core::ports::LockSensor;
use crate::util::Event;

/// Whether the app should be querying and uploading. `on_change()` is called when that, or why, changes.
pub struct Power {
    sensor: Arc<dyn LockSensor>,
    on_change: Callback,
    on_lock_change: Arc<dyn Fn(bool) + Send + Sync>, // (locked): dim the backlight, put the view back
    wake: Arc<Event>,
    stop: Arc<Event>,
    save: Callback,
    paused: AtomicBool, // manual pause (not remembered: a forgotten pause would be a nasty surprise)
    locked: AtomicBool,
    pause_on_lock: AtomicBool, // stop while the computer is locked
}

impl Power {
    pub fn new(sensor: Arc<dyn LockSensor>, on_change: Callback, on_lock_change: Arc<dyn Fn(bool) + Send + Sync>, wake: Arc<Event>, stop: Arc<Event>, save: Callback) -> Self {
        Self { sensor, on_change, on_lock_change, wake, stop, save, paused: AtomicBool::new(false), locked: AtomicBool::new(false), pause_on_lock: AtomicBool::new(true) }
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
    pub fn locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }
    pub fn pause_on_lock(&self) -> bool {
        self.pause_on_lock.load(Ordering::SeqCst)
    }

    pub fn is_paused(&self) -> bool {
        self.paused() || (self.pause_on_lock() && self.locked())
    }

    pub fn toggle_pause(&self) {
        let now = !self.paused.fetch_xor(true, Ordering::SeqCst);
        log::info!("{}", if now { "paused" } else { "resumed" });
        (self.on_change)();
        if !now {
            self.wake.set(); // catch up at once
        }
    }

    pub fn toggle_pause_on_lock(&self) {
        let now = !self.pause_on_lock.fetch_xor(true, Ordering::SeqCst);
        log::info!("pause when locked {}", if now { "on" } else { "off" });
        (self.save)();
        (self.on_change)();
        self.wake.set();
    }

    /// Thread: notice the computer being locked or unlocked, within a few seconds.
    pub fn watch_lock(&self) {
        while !self.stop.is_set() {
            let locked = self.sensor.is_locked();
            if locked != self.locked() {
                self.set_locked(locked);
            }
            self.stop.wait(config::LOCK_POLL);
        }
    }

    pub fn set_locked(&self, locked: bool) {
        self.locked.store(locked, Ordering::SeqCst);
        log::info!("computer {}", if locked { "locked" } else { "unlocked" });
        (self.on_change)();
        (self.on_lock_change)(locked);
        if !locked {
            self.wake.set(); // refresh at once and put the view back
        }
    }
}

impl Part for Power {
    fn name(&self) -> &'static str {
        "power"
    }

    fn restore(&self, saved: &Value) {
        self.pause_on_lock.store(saved.get("pause_on_lock").and_then(Value::as_bool).unwrap_or(true), Ordering::SeqCst);
    }

    fn snapshot(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("pause_on_lock".into(), json!(self.pause_on_lock()));
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Sensor(AtomicBool);
    impl LockSensor for Sensor {
        fn is_locked(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn power() -> (Arc<Power>, Arc<AtomicUsize>, Arc<std::sync::Mutex<Vec<bool>>>, Arc<Event>, Arc<Sensor>) {
        let changes = Arc::new(AtomicUsize::new(0));
        let locks = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (c, l) = (changes.clone(), locks.clone());
        let (wake, sensor) = (Arc::new(Event::new()), Arc::new(Sensor(AtomicBool::new(false))));
        let p = Arc::new(Power::new(
            sensor.clone(),
            Arc::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
            Arc::new(move |locked| l.lock().unwrap().push(locked)),
            wake.clone(),
            Arc::new(Event::new()),
            Arc::new(|| {}),
        ));
        (p, changes, locks, wake, sensor)
    }

    #[test]
    fn manual_pause_and_pause_on_lock() {
        let (p, changes, _, wake, _) = power();
        assert!(!p.is_paused());
        p.toggle_pause();
        assert!(p.is_paused() && !wake.is_set());
        p.toggle_pause();
        assert!(!p.is_paused() && wake.is_set(), "resuming catches up at once");
        p.set_locked(true);
        assert!(p.is_paused(), "locked and pausing on lock");
        p.toggle_pause_on_lock();
        assert!(!p.is_paused(), "locked but not pausing on lock");
        assert_eq!(changes.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn the_lock_watcher_notices_a_lock_and_an_unlock() {
        let (p, _, locks, _, sensor) = power();
        let stop = p.stop.clone();
        let watcher = p.clone();
        let t = std::thread::spawn(move || watcher.watch_lock());
        sensor.0.store(true, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(p.locked());
        stop.set();
        t.join().unwrap();
        assert_eq!(*locks.lock().unwrap(), [true]);
    }

    #[test]
    fn pause_on_lock_is_remembered_and_a_manual_pause_is_not() {
        let (p, ..) = power();
        p.toggle_pause();
        p.restore(&json!({"pause_on_lock": false}));
        assert!(!p.pause_on_lock());
        assert_eq!(p.snapshot()["pause_on_lock"], false);
        assert!(!p.snapshot().contains_key("paused"));
        p.restore(&json!({}));
        assert!(p.pause_on_lock());
    }
}
