//! Usage thresholds: when a bar changes colour, and when to send a desktop notification.

pub const WARN_PCT: f64 = 70.0; // the bar and percentage turn yellow from here
pub const CRIT_PCT: f64 = 90.0; // ... and red from here
pub const NOTIFY_PCTS: [f64; 2] = [80.0, 95.0]; // notify the first time usage reaches each of these
pub const RESET_DROP: f64 = 30.0; // usage falling by this many points can only mean the window reset
pub const HYSTERESIS: f64 = 5.0; // points below a threshold before it can notify again

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarState {
    Ok,
    Warn,
    Crit,
}

pub fn bar_state(pct: Option<f64>) -> BarState {
    match pct {
        None => BarState::Ok,
        Some(p) if p >= CRIT_PCT => BarState::Crit,
        Some(p) if p >= WARN_PCT => BarState::Warn,
        Some(_) => BarState::Ok,
    }
}

/// How many NOTIFY_PCTS `pct` has reached (0, 1 or 2).
pub fn level(pct: Option<f64>) -> usize {
    pct.map_or(0, |p| NOTIFY_PCTS.iter().filter(|t| p >= **t).count())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// Usage newly reached this notify threshold.
    Rise(f64),
    /// It dropped far enough to mean the window reset after an alert was raised.
    Reset,
}

/// Compare a new reading with the previous one: (new level, event).
pub fn evaluate(prev_level: usize, prev_pct: Option<f64>, pct: Option<f64>) -> (usize, Option<Event>) {
    let Some(p) = pct else { return (prev_level, None) };
    let new = level(pct);
    if new > prev_level {
        return (new, Some(Event::Rise(NOTIFY_PCTS[new - 1])));
    }
    if new < prev_level {
        if prev_pct.is_some_and(|pp| pp - p >= RESET_DROP) {
            return (new, Some(Event::Reset));
        }
        // a small dip below a threshold doesn't re-arm it, or readings hovering around it would keep notifying
        let rearm_below = NOTIFY_PCTS[prev_level - 1] - HYSTERESIS;
        return (if p < rearm_below { new } else { prev_level }, None);
    }
    (new, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states() {
        use BarState::*;
        let s = bar_state;
        assert_eq!(
            [s(None), s(Some(0.0)), s(Some(69.9)), s(Some(70.0)), s(Some(89.9)), s(Some(90.0)), s(Some(100.0))],
            [Ok, Ok, Ok, Warn, Warn, Crit, Crit]
        );
    }

    #[test]
    fn levels() {
        let l: Vec<usize> = [None, Some(0.0), Some(79.0), Some(80.0), Some(94.0), Some(95.0), Some(100.0)].map(level).to_vec();
        assert_eq!(l, [0, 0, 0, 1, 1, 2, 2]);
    }

    #[test]
    fn no_event_below_threshold_or_without_reading() {
        assert_eq!(evaluate(0, None, Some(50.0)), (0, None));
        assert_eq!(evaluate(1, Some(85.0), None), (1, None));
    }

    #[test]
    fn rise_notifies_once_per_threshold() {
        assert_eq!(evaluate(0, Some(50.0), Some(82.0)), (1, Some(Event::Rise(80.0))));
        assert_eq!(evaluate(1, Some(82.0), Some(90.0)), (1, None));
        assert_eq!(evaluate(1, Some(90.0), Some(96.0)), (2, Some(Event::Rise(95.0))));
        assert_eq!(evaluate(0, None, Some(96.0)), (2, Some(Event::Rise(95.0))));
    }

    #[test]
    fn reset_after_an_alert() {
        assert_eq!(evaluate(2, Some(96.0), Some(3.0)), (0, Some(Event::Reset)));
        assert_eq!(evaluate(1, Some(82.0), Some(50.0)), (0, Some(Event::Reset)));
        assert_eq!(evaluate(0, Some(60.0), Some(2.0)), (0, None));
    }

    #[test]
    fn hovering_around_a_threshold_does_not_renotify() {
        let (level, event) = evaluate(1, Some(82.0), Some(78.0));
        assert_eq!((level, event), (1, None));
        assert_eq!(evaluate(level, Some(78.0), Some(81.0)), (1, None));
        assert_eq!(evaluate(1, Some(81.0), Some(74.0)), (0, None));
        assert_eq!(evaluate(0, Some(74.0), Some(80.0)), (1, Some(Event::Rise(80.0))));
    }
}
