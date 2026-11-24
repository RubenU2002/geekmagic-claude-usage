//! Small shared helpers: clocks, a lock that ignores poisoning, a settable event, "run this in a thread".

use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Seconds since the program started, never going back: what `time.monotonic()` is for the Python version.
pub fn mono() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() + 1000.0 // (never 0: "not before 0" checks stay meaningful)
}

/// Seconds since 1970.
pub fn epoch() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub trait LockExt<T> {
    /// Lock, carrying on if another thread panicked while holding it (the data is plain state, still usable).
    fn lk(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn lk(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Python's `round()`: halves go to the even neighbour.
pub fn pyround(x: f64) -> i64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && (r as i64) % 2 != 0 {
        (r - x.signum()) as i64
    } else {
        r as i64
    }
}

/// A flag threads can wait on, like `threading.Event`.
#[derive(Default)]
pub struct Event {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl Event {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self) {
        *self.flag.lk() = true;
        self.cv.notify_all();
    }
    pub fn clear(&self) {
        *self.flag.lk() = false;
    }
    pub fn is_set(&self) -> bool {
        *self.flag.lk()
    }
    /// Wait until set or `secs` pass. True if it was set.
    pub fn wait(&self, secs: f64) -> bool {
        let deadline = Instant::now() + Duration::from_secs_f64(secs.clamp(0.0, 86_400.0 * 365.0));
        let mut flag = self.flag.lk();
        while !*flag {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            flag = self.cv.wait_timeout(flag, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
        true
    }
}

/// Run something off the caller's thread (small stack: these only call into other code).
pub fn in_background<F: FnOnce() + Send + 'static>(f: F) {
    let _ = std::thread::Builder::new().stack_size(512 * 1024).spawn(f);
}

pub fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyround_goes_to_even() {
        assert_eq!([0.5, 1.5, 2.5, 27.5, 24.5, -0.5, 2.4, 2.6].map(pyround), [0, 2, 2, 28, 24, 0, 2, 3]);
    }

    #[test]
    fn event_waits_and_wakes() {
        let e = std::sync::Arc::new(Event::new());
        assert!(!e.wait(0.01));
        let e2 = e.clone();
        let t = std::thread::spawn(move || e2.wait(5.0));
        std::thread::sleep(Duration::from_millis(30));
        e.set();
        assert!(t.join().unwrap());
        e.clear();
        assert!(!e.is_set());
    }
}
