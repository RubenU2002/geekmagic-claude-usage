//! Pace projection: at the rate you're going, where will a usage window end up, and when would it run out?

use chrono::{DateTime, Utc};

use super::model::{Projection, Usage, WindowKind};
use crate::util::pyround;

/// Window lengths in minutes. Codex reports its own (windowDurationMins); Claude's /usage doesn't, so these apply.
pub fn default_window_min(kind: WindowKind) -> f64 {
    match kind {
        WindowKind::Current => 300.0,
        WindowKind::Weekly => 10080.0,
    }
}

pub const MIN_ELAPSED: f64 = 0.08; // too early in a window to say anything useful (fraction of the window)
pub const MIN_PCT: f64 = 2.0; // likewise when almost nothing has been used
pub const MIN_SPAN_MIN: f64 = 5.0; // recent readings must span at least this long to give a rate...
pub const MIN_SPAN_SHARE: f64 = 0.25; // ...and at least this share of the look-back

/// How far back "your recent pace" looks (the week: the last day).
pub fn lookback_min(kind: WindowKind) -> f64 {
    match kind {
        WindowKind::Current => 30.0,
        WindowKind::Weekly => 1440.0,
    }
}

/// Projection for one window, or None when it's too early / there's nothing to go on. The pace is the faster of your
/// average since the window started and your recent rate, so a burst of work shows up instead of being averaged away.
pub fn project(
    pct: Option<f64>,
    reset_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    window_min: f64,
    recent_rate: Option<f64>,
) -> Option<Projection> {
    let (pct, reset_at) = (pct?, reset_at?);
    if pct < MIN_PCT {
        return None;
    }
    let remaining = (reset_at - now).num_milliseconds() as f64 / 60_000.0;
    let elapsed = window_min - remaining;
    if remaining <= 0.0 || elapsed <= 0.0 || elapsed / window_min < MIN_ELAPSED {
        return None;
    }
    let rate = (pct / elapsed).max(recent_rate.unwrap_or(0.0));
    let end = pct + rate * remaining;
    let hits = end >= 100.0 && pct < 100.0;
    Some(Projection { rate, end, hits_limit: hits, minutes_to_limit: hits.then(|| (100.0 - pct) / rate) })
}

/// Percentage points per minute over the last `lookback_min`, from [epoch, pct, reset_epoch] readings of the current
/// window. None if there isn't enough history (or it was flat/falling).
pub fn recent_rate(history: &[[f64; 3]], reset_epoch: Option<f64>, now_epoch: f64, lookback_min: f64) -> Option<f64> {
    let reset = reset_epoch?;
    let points: Vec<(f64, f64)> = history
        .iter()
        .filter(|[t, _, r]| (r - reset).abs() < 120.0 && now_epoch - t <= lookback_min * 60.0)
        .map(|[t, p, _]| (*t, *p))
        .collect();
    if points.len() < 2 {
        return None;
    }
    let cmp = |a: &&(f64, f64), b: &&(f64, f64)| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1));
    let (t0, p0) = *points.iter().min_by(cmp)?;
    let (t1, p1) = *points.iter().max_by(cmp)?;
    let span = (t1 - t0) / 60.0;
    if span < MIN_SPAN_MIN.max(lookback_min * MIN_SPAN_SHARE) || p1 <= p0 {
        return None;
    }
    Some((p1 - p0) / span)
}

/// Compact duration: "45m", "2h 10m", "3d 4h".
pub fn format_minutes(minutes: f64) -> String {
    let minutes = pyround(minutes).max(0);
    let (days, rest) = (minutes / 1440, minutes % 1440);
    let (hours, mins) = (rest / 60, rest % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaceState {
    Ok,
    Warn,
    Crit,
}

/// (text, state) for the screen: "Full in 1h 30m" (crit under an hour, else warn) or "~62% at reset" (muted).
pub fn describe(proj: Option<&Projection>) -> Option<(String, PaceState)> {
    let proj = proj?;
    if proj.hits_limit {
        let m = proj.minutes_to_limit.unwrap_or(0.0);
        return Some((format!("Full in {}", format_minutes(m)), if m < 60.0 { PaceState::Crit } else { PaceState::Warn }));
    }
    Some((format!("~{}% at reset", pyround(proj.end)), PaceState::Ok))
}

/// Set usage.pace for each window (in place) and return `usage`.
pub fn annotate(usage: &mut Usage, recent: [Option<f64>; 2]) {
    let now = usage.now.with_timezone(&Utc);
    for kind in WindowKind::ALL {
        let w = usage.window(kind);
        let minutes = w.minutes.filter(|m| *m != 0.0).unwrap_or_else(|| default_window_min(kind));
        let projection = project(w.pct, w.reset, now, minutes, recent[kind.index()]);
        usage.pace[kind.index()] = projection;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Window;
    use chrono::{Duration, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 2, 14, 57, 0).unwrap()
    }
    fn mins(m: i64) -> Option<DateTime<Utc>> {
        Some(now() + Duration::minutes(m))
    }
    fn usage(current: Option<f64>, left_min: Option<i64>, weekly: Option<f64>, weekly_left_h: Option<i64>, window_min: Option<f64>) -> Usage {
        let n = now().with_timezone(&chrono::Local);
        Usage::new(
            "Claude",
            Window { pct: current, reset: left_min.and_then(mins), minutes: window_min },
            Window { pct: weekly, reset: weekly_left_h.map(|h| now() + Duration::hours(h)), minutes: None },
            n,
        )
    }

    #[test]
    fn steady_pace_projects_to_the_end_of_the_window() {
        let p = project(Some(62.0), mins(90), now(), 300.0, None).unwrap();
        assert!((p.end - (62.0 + 62.0 / 210.0 * 90.0)).abs() < 1e-3);
        assert!(!p.hits_limit && p.minutes_to_limit.is_none());
    }

    #[test]
    fn heading_past_100_says_when() {
        let p = project(Some(80.0), mins(180), now(), 300.0, None).unwrap();
        assert!(p.hits_limit);
        assert!((p.minutes_to_limit.unwrap() - 30.0).abs() < 1e-3);
    }

    #[test]
    fn too_early_or_too_little_gives_nothing() {
        assert!(project(Some(30.0), mins(290), now(), 300.0, None).is_none());
        assert!(project(Some(1.0), mins(100), now(), 300.0, None).is_none());
        assert!(project(None, mins(100), now(), 300.0, None).is_none());
        assert!(project(Some(50.0), None, now(), 300.0, None).is_none());
        assert!(project(Some(50.0), mins(-1), now(), 300.0, None).is_none());
        assert!(!project(Some(100.0), mins(60), now(), 300.0, None).unwrap().hits_limit);
    }

    #[test]
    fn a_recent_burst_beats_the_average_but_a_slow_one_does_not_hide_a_heavy_start() {
        let calm = project(Some(40.0), mins(150), now(), 300.0, None).unwrap();
        let burst = project(Some(40.0), mins(150), now(), 300.0, Some(0.8)).unwrap();
        assert!(burst.rate > calm.rate && burst.rate == 0.8 && burst.hits_limit);
        let slow = project(Some(40.0), mins(150), now(), 300.0, Some(0.01)).unwrap();
        assert!((slow.rate - 40.0 / 150.0).abs() < 1e-9);
    }

    const RESET: f64 = 1_800_000_000.0;
    fn hist(points: &[(f64, f64)]) -> Vec<[f64; 3]> {
        points.iter().map(|(t, p)| [*t, *p, RESET]).collect()
    }

    #[test]
    fn recent_rate_over_the_lookback() {
        let now = 1_000_000.0;
        let h = hist(&[(now - 1500.0, 10.0), (now - 900.0, 14.0), (now, 20.0)]);
        assert!((recent_rate(&h, Some(RESET), now, 30.0).unwrap() - 10.0 / 25.0).abs() < 1e-9);
    }

    #[test]
    fn recent_rate_needs_enough_history() {
        let now = 1_000_000.0;
        assert!(recent_rate(&[], Some(RESET), now, 30.0).is_none());
        assert!(recent_rate(&hist(&[(now, 20.0)]), Some(RESET), now, 30.0).is_none());
        assert!(recent_rate(&hist(&[(now - 120.0, 18.0), (now, 20.0)]), Some(RESET), now, 30.0).is_none());
        let burst = hist(&[(now - 1200.0, 17.0), (now, 20.0)]);
        assert!(recent_rate(&burst, Some(RESET), now, lookback_min(WindowKind::Weekly)).is_none());
        assert!((recent_rate(&burst, Some(RESET), now, lookback_min(WindowKind::Current)).unwrap() - 3.0 / 20.0).abs() < 1e-9);
        let day = hist(&[(now - 8.0 * 3600.0, 12.0), (now - 4.0 * 3600.0, 16.0), (now, 20.0)]);
        assert!((recent_rate(&day, Some(RESET), now, lookback_min(WindowKind::Weekly)).unwrap() - 8.0 / 480.0).abs() < 1e-9);
    }

    #[test]
    fn flat_falling_other_window_or_too_old_readings_are_ignored() {
        let now = 1_000_000.0;
        assert!(recent_rate(&hist(&[(now - 900.0, 20.0), (now, 20.0)]), Some(RESET), now, 30.0).is_none());
        assert!(recent_rate(&hist(&[(now - 900.0, 50.0), (now, 3.0)]), Some(RESET), now, 30.0).is_none());
        let old = vec![[now - 900.0, 90.0, RESET - 18000.0], [now - 800.0, 95.0, RESET - 18000.0], [now, 4.0, RESET]];
        assert!(recent_rate(&old, Some(RESET), now, 30.0).is_none());
        let h = hist(&[(now - 7200.0, 1.0), (now - 600.0, 10.0), (now, 12.0)]);
        assert!((recent_rate(&h, Some(RESET), now, 30.0).unwrap() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn formats_and_texts() {
        let f: Vec<String> = [0.0, 7.0, 59.0, 60.0, 130.0, 1439.0, 1440.0, 3.0 * 1440.0 + 270.0].map(format_minutes).to_vec();
        assert_eq!(f, ["0m", "7m", "59m", "1h 0m", "2h 10m", "23h 59m", "1d 0h", "3d 4h"]);
        let calm = project(Some(62.0), mins(90), now(), 300.0, None);
        assert_eq!(describe(calm.as_ref()), Some(("~89% at reset".into(), PaceState::Ok)));
        let soon = project(Some(80.0), mins(180), now(), 300.0, None);
        assert_eq!(describe(soon.as_ref()), Some(("Full in 30m".into(), PaceState::Crit)));
        let later = project(Some(30.0), mins(240), now(), 300.0, None);
        assert_eq!(describe(later.as_ref()), Some(("Full in 2h 20m".into(), PaceState::Warn)));
        assert_eq!(describe(None), None);
    }

    #[test]
    fn annotate_uses_each_windows_length_and_recent_rates() {
        let mut u = usage(Some(62.0), Some(90), Some(95.0), Some(24), None);
        annotate(&mut u, [None, None]);
        assert!(!u.pace[0].unwrap().hits_limit);
        assert!(u.pace[1].unwrap().hits_limit);
        let mut calm = usage(None, None, Some(70.0), Some(24), None);
        annotate(&mut calm, [None, None]);
        let c = calm.pace[1].unwrap();
        assert!(!c.hits_limit && (c.end - (70.0 + 70.0 / 144.0 * 24.0)).abs() < 1e-3);
        let (mut a, mut b) = (usage(Some(62.0), Some(90), None, None, None), usage(Some(62.0), Some(90), None, None, Some(600.0)));
        annotate(&mut a, [None, None]);
        annotate(&mut b, [None, None]);
        assert_ne!(a.pace[0].unwrap().rate, b.pace[0].unwrap().rate);
        let mut r = usage(Some(40.0), Some(150), None, None, None);
        annotate(&mut r, [Some(0.9), None]);
        assert_eq!(r.pace[0].unwrap().rate, 0.9);
        let mut none = usage(None, None, None, None, None);
        annotate(&mut none, [None, None]);
        assert_eq!(none.pace, [None, None]);
    }
}
