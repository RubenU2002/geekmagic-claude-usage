//! The pieces the screens are drawn from: bars, pills, the pace line, the "working" tag, share rows, the week chart.

use chrono::{DateTime, Local, Timelike, Utc};

use super::canvas::{text_bbox, text_width, Canvas};
use super::mascots::{draw_mascot, theme_of};
use super::palette::*;
use crate::core::activity::{self as activity_stats, DayCount, Named};
use crate::core::alerts;
use crate::core::model::{Usage, WindowKind};
use crate::core::pace::{self, PaceState};
use crate::core::themes::{blend as mix, Rgb, CODEX};
use crate::util::pyround;

/// Font sizes (pixels per em) a screen uses.
#[derive(Debug, Clone, Copy)]
pub struct Fonts {
    pub title: f32,
    pub body: f32,
    pub small: f32,
    pub tiny: f32,
    pub pct: f32,
}

/// 12-hour clock time, "2:57 PM". Built by hand, never from the system locale.
pub fn clock(moment: DateTime<Local>) -> String {
    let h = moment.hour();
    format!("{}:{:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, moment.minute(), if h < 12 { "AM" } else { "PM" })
}

pub fn format_delta(reset_at: Option<DateTime<Utc>>, now: DateTime<Local>) -> String {
    let Some(reset) = reset_at else { return "Resets in --".into() };
    let total_minutes = ((reset - now.with_timezone(&Utc)).num_seconds().div_euclid(60)).max(0);
    let (days, rem) = (total_minutes / (24 * 60), total_minutes % (24 * 60));
    let (hours, minutes) = (rem / 60, rem % 60);
    if days >= 1 {
        format!("Resets in {days}d")
    } else {
        format!("Resets in {hours}h {minutes}m")
    }
}

/// (text, colour) for the free-resets note in the corner of Codex's screen: "3 resets · 2d" (days until the next one
/// expires; yellow from 3 days, red from 1 so it gets used in time).
pub fn resets_chip(usage: &Usage) -> Option<(String, Rgb)> {
    let extra = usage.codex_extra.as_ref();
    let count = extra.map_or(0, |e| e.free_resets);
    if usage.title != "Codex" || count == 0 {
        return None;
    }
    let days = extra.and_then(|e| e.next_reset_expires_days);
    let text = format!("{count} reset{}{}", if count == 1 { "" } else { "s" }, days.map_or(String::new(), |d| format!(" · {d}d")));
    let color = match days {
        Some(d) if d <= 1 => CRIT,
        Some(d) if d <= 3 => WARN,
        _ => CODEX.weekly,
    };
    Some((text, color))
}

// --- Stats view: activity numbers for both providers -----------------------------------------------------

/// Requests per day for the last week: one bar per day (today's in full colour, earlier days dimmer), the number above
/// each bar and the weekday under it. Scaled to the provider's own busiest day, so the shapes compare.
pub fn draw_week_chart(c: &mut Canvas, days: &[DayCount], left: i32, right: i32, baseline: i32, height: i32, color: Rgb, fonts: &Fonts) {
    let slot = f64::from(right - left) / days.len() as f64;
    let top_value = days.iter().map(|d| d.requests).max().unwrap_or(0);
    let dim = mix(color, BG, 0.45);
    for (i, day) in days.iter().enumerate() {
        let n = day.requests;
        let x = f64::from(left) + i as f64 * slot;
        let bar_w = 20f64.min(slot - 6.0);
        let x0 = pyround(x + (slot - bar_w) / 2.0) as i32;
        let h = if top_value > 0 && n > 0 { 2.max(pyround(f64::from(height) * n as f64 / top_value as f64) as i32) } else { 2 };
        let today = i == days.len() - 1;
        let fill = if today && n > 0 { color } else if n > 0 { dim } else { PILL_BG };
        c.rect(x0, baseline - h, x0 + pyround(bar_w) as i32 - 1, baseline - 1, fill);
        let label = if n < 10_000 { activity_stats::thousands(n) } else { format!("{}k", n / 1000) };
        let w = text_width(&label, fonts.tiny);
        let ink = if today { TEXT } else { MUTED };
        c.text(f64::from(x0) + bar_w / 2.0 - w / 2.0, f64::from(baseline - h - 11), &label, fonts.tiny, ink);
        let name = activity_stats::weekday(&day.date);
        let w = text_width(name, fonts.tiny);
        c.text(f64::from(x0) + bar_w / 2.0 - w / 2.0, f64::from(baseline + 1), name, fonts.tiny, ink);
    }
}

/// (label, colour) of the badge for an agent that is waiting for you or working, else None. Waiting wins.
pub fn busy_style(usage: &Usage) -> Option<(&'static str, Rgb)> {
    if usage.waiting {
        Some(("Waiting", WAITING_COLOR))
    } else if usage.working {
        Some(("Working", WORKING_COLOR))
    } else {
        None
    }
}

/// A small dot with a halo: "this agent is busy" (green) or "is waiting for you" (amber), in a panel header.
pub fn draw_working_dot(c: &mut Canvas, x: f64, y: f64, color: Rgb) {
    let (x, y) = (x as i32, y as i32);
    c.ellipse(x - 5, y - 5, x + 5, y + 5, mix(color, BG, 0.3));
    c.ellipse(x - 3, y - 3, x + 3, y + 3, color);
}

/// In a panel header, the dot and its word ("Working" / "Waiting"). By default it's right-aligned on the line under the
/// clock; with `left_of` it sits just left of that x instead, for headers with room. Nothing if the agent isn't busy.
pub fn draw_busy_tag(c: &mut Canvas, y: f64, usage: &Usage, font: f32, left_of: Option<f64>) {
    let Some((label, color)) = busy_style(usage) else { return };
    let width = text_width(label, font);
    match left_of {
        None => {
            draw_working_dot(c, 230.0 - width - 11.0, y + 7.0, color);
            c.text(230.0 - width, y, label, font, color);
        }
        Some(x) => {
            draw_working_dot(c, x - 8.0, y + 7.0, color);
            c.text(x - 16.0 - width, y, label, font, color);
        }
    }
}

/// The header every two-provider view shares: small mascot, name, time (or the working dot beside it).
pub fn draw_panel_header(c: &mut Canvas, top: i32, usage: &Usage, fonts: &Fonts) {
    let theme = theme_of(usage);
    draw_mascot(c, 10, top + 8, 2, theme);
    c.text(46.0, f64::from(top + 5), &usage.title, fonts.title, TEXT);
    if !usage.stale {
        let updated = clock(usage.now);
        let width = text_width(&updated, fonts.small);
        c.text(230.0 - width, f64::from(top + 10), &updated, fonts.small, theme.current);
        draw_busy_tag(c, f64::from(top + 10), usage, fonts.small, Some(230.0 - width));
    }
}

/// A panel whose numbers couldn't be refreshed: everything dimmed, and since when.
pub fn dim_stale_panel(c: &mut Canvas, top: i32, usage: &Usage, fonts: &Fonts) {
    if !usage.stale {
        return;
    }
    c.blend_box(0, top as usize, WIDTH, (top + SPLIT_PANEL_H) as usize, BG, 0.55);
    let text = format!("! stale since {}", clock(usage.now));
    c.text(230.0 - text_width(&text, fonts.small), f64::from(top + 10), &text, fonts.small, WARN);
}

/// Up to three rows: a name, its share in %, and a bar under them.
pub fn draw_share_rows(c: &mut Canvas, items: &[Named], total: usize, left: i32, width: i32, top: i32, color: Rgb, fonts: &Fonts) {
    for (i, (name, share)) in activity_stats::shares(items, total, 3).into_iter().enumerate() {
        let y = top + i as i32 * 25;
        let pct = if share >= 0.01 { format!("{}%", pyround(share * 100.0)) } else { "<1%".to_string() };
        let pct_w = text_width(&pct, fonts.tiny);
        let name = Canvas::fit(&name, fonts.tiny, f64::from(width) - pct_w - 6.0);
        c.text(f64::from(left), f64::from(y), &name, fonts.tiny, TEXT);
        c.text(f64::from(left + width) - pct_w, f64::from(y), &pct, fonts.tiny, MUTED);
        c.rounded_rect(left, y + 14, left + width, y + 19, 2, PILL_BG);
        c.rounded_rect(left, y + 14, left + 3.max(pyround(f64::from(width) * share) as i32), y + 19, 2, color);
    }
}

/// Everything `draw_section` needs about one usage window of the single-provider screen.
pub struct Section<'a> {
    pub top: i32,
    pub label: &'a str,
    pub percent: Option<f64>,
    pub reset_text: String,
    pub accent: Rgb,
    pub pace_info: Option<(String, PaceState)>,
}

pub const PILL_FONT: f32 = 14.0;
pub const PCT_FONT: f32 = 30.0;
pub const RESET_FONT: f32 = 14.0;
pub const FOOTER_FONT: f32 = 13.0;

fn pct_text(percent: Option<f64>) -> String {
    percent.map_or("--%".to_string(), |p| format!("{}%", pyround(p)))
}

fn truthy(percent: Option<f64>) -> bool {
    percent.is_some_and(|p| p != 0.0)
}

pub fn draw_section(c: &mut Canvas, s: &Section) {
    let (panel_left, panel_right, pad) = (10, 230, 4);
    let state = alerts::bar_state(s.percent);
    let accent = state_color(state).unwrap_or(s.accent);
    c.text(f64::from(panel_left + pad), f64::from(s.top + 6), &pct_text(s.percent), PCT_FONT, state_color(state).unwrap_or(TEXT));

    let bbox = text_bbox(s.label, PILL_FONT);
    let pill_text_w = bbox.2 - bbox.0;
    let (pill_h, pill_w) = (26, pill_text_w + 28);
    let pill_left = panel_right - pad - pill_w;
    let pill_top = s.top + 10;
    c.rounded_rect(pill_left, pill_top, pill_left + pill_w, pill_top + pill_h, pill_h / 2, PILL_BG);
    c.text(
        f64::from(pill_left + (pill_w - pill_text_w) / 2),
        f64::from(pill_top + (pill_h - bbox.3) / 2),
        s.label,
        PILL_FONT,
        TEXT,
    );

    let (bar_left, bar_top, bar_right, bar_h) = (panel_left + pad, s.top + 44, panel_right - pad, 18);
    c.rounded_rect(bar_left, bar_top, bar_right, bar_top + bar_h, 9, PILL_BG);
    if truthy(s.percent) {
        let fill_w = pyround(f64::from(bar_right - bar_left) * s.percent.unwrap_or(0.0).min(100.0) / 100.0) as i32;
        if fill_w > 0 {
            c.rounded_rect(bar_left, bar_top, bar_left + fill_w, bar_top + bar_h, 9.min(3.max(fill_w / 2)), accent);
        }
    }

    c.text(f64::from(panel_left + pad), f64::from(s.top + 68), &s.reset_text, RESET_FONT, MUTED);
    let left_end = f64::from(panel_left + pad) + text_width(&s.reset_text, RESET_FONT);
    draw_pace(c, s.pace_info.as_ref(), f64::from(panel_right - pad), f64::from(s.top + 70), left_end, FOOTER_FONT);
}

/// The projection ("Full in 1h 30m" / "~62% at reset"), right-aligned; skipped if it wouldn't fit next to the reset
/// countdown, which matters more.
pub fn draw_pace(c: &mut Canvas, info: Option<&(String, PaceState)>, right: f64, y: f64, left_end: f64, font: f32) {
    let Some((text, state)) = info else { return };
    let width = text_width(text, font);
    if left_end + 10.0 < right - width {
        let color = match state {
            PaceState::Ok => MUTED,
            PaceState::Warn => WARN,
            PaceState::Crit => CRIT,
        };
        c.text(right - width, y, text, font, color);
    }
}

/// (text, state) projection for a usage window, or None (also None while the numbers are stale).
pub fn window_pace(usage: &Usage, window: WindowKind) -> Option<(String, PaceState)> {
    if usage.stale {
        return None;
    }
    pace::describe(usage.pace_of(window))
}

/// One compact usage row: big number + bar on the first line, label + reset countdown under it.
pub fn draw_split_row(c: &mut Canvas, top: i32, label: &str, percent: Option<f64>, reset_text: &str, accent: Rgb, fonts: &Fonts, pace_info: Option<&(String, PaceState)>) {
    let state = alerts::bar_state(percent);
    let color = state_color(state).unwrap_or(accent);
    c.text(10.0, f64::from(top - 1), &pct_text(percent), fonts.pct, state_color(state).unwrap_or(TEXT));
    let (bar_left, bar_right, bar_top, bar_h) = (68, 230, top + 4, 12);
    c.rounded_rect(bar_left, bar_top, bar_right, bar_top + bar_h, 6, PILL_BG);
    if truthy(percent) {
        let fill_w = pyround(f64::from(bar_right - bar_left) * percent.unwrap_or(0.0).min(100.0) / 100.0) as i32;
        if fill_w > 0 {
            c.rounded_rect(bar_left, bar_top, bar_left + fill_w, bar_top + bar_h, 6.min(2.max(fill_w / 2)), color);
        }
    }
    c.text(10.0, f64::from(top + 26), label, fonts.small, MUTED);
    c.text(f64::from(bar_left), f64::from(top + 26), reset_text, fonts.small, MUTED);
    let left_end = f64::from(bar_left) + text_width(reset_text, fonts.small);
    draw_pace(c, pace_info, 230.0, f64::from(top + 26), left_end, fonts.small);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{CodexExtra, Window};
    use chrono::{Duration, TimeZone};

    #[test]
    fn clock_is_twelve_hour_and_locale_free() {
        let t = |h, m| clock(Local.with_ymd_and_hms(2026, 10, 2, h, m, 0).unwrap());
        assert_eq!((t(14, 57), t(0, 5), t(12, 0), t(9, 0)), ("2:57 PM".into(), "12:05 AM".into(), "12:00 PM".into(), "9:00 AM".into()));
    }

    #[test]
    fn countdown_text() {
        let now = Local.with_ymd_and_hms(2026, 10, 2, 14, 0, 0).unwrap();
        let at = |m| format_delta(Some(now.with_timezone(&Utc) + Duration::minutes(m)), now);
        assert_eq!(at(0), "Resets in 0h 0m");
        assert_eq!(at(130), "Resets in 2h 10m");
        assert_eq!(at(1500), "Resets in 1d");
        assert_eq!(at(-5), "Resets in 0h 0m");
        assert_eq!(format_delta(None, now), "Resets in --");
    }

    #[test]
    fn resets_chip_colours_follow_the_days_left() {
        let mut u = Usage::new("Codex", Window::default(), Window::default(), Local::now());
        assert!(resets_chip(&u).is_none());
        u.codex_extra = Some(CodexExtra { free_resets: 3, next_reset_expires_days: Some(2) });
        assert_eq!(resets_chip(&u), Some(("3 resets · 2d".into(), WARN)));
        u.codex_extra = Some(CodexExtra { free_resets: 1, next_reset_expires_days: Some(1) });
        assert_eq!(resets_chip(&u), Some(("1 reset · 1d".into(), CRIT)));
        u.codex_extra = Some(CodexExtra { free_resets: 2, next_reset_expires_days: None });
        assert_eq!(resets_chip(&u), Some(("2 resets".into(), CODEX.weekly)));
        u.title = "Claude".into();
        assert!(resets_chip(&u).is_none());
    }

    #[test]
    fn busy_badge_prefers_waiting() {
        let mut u = Usage::new("Claude", Window::default(), Window::default(), Local::now());
        assert!(busy_style(&u).is_none());
        u.working = true;
        assert_eq!(busy_style(&u).unwrap().0, "Working");
        u.waiting = true;
        assert_eq!(busy_style(&u).unwrap().0, "Waiting");
    }
}
