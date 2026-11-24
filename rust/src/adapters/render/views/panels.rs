//! The views of two providers' local activity: stats, projects and models, peak hours.

use super::super::canvas::Canvas;
use super::super::components::*;
use super::super::mascots::theme_of;
use super::super::palette::*;
use crate::core::activity as activity_stats;
use crate::core::model::{Activity, Usage};
use crate::core::themes::blend;
use crate::util::pyround;

pub const FONTS: Fonts = Fonts { title: 17.0, body: 12.0, small: 11.0, tiny: 10.0, pct: 22.0 };

/// What a panel says when there is nothing to chart yet.
fn nothing_to_show(activity: &Activity, no_activity: &str) -> &'static str {
    match activity {
        Activity::NoLogs => "No local activity logs found",
        Activity::Counting => "Counting...",
        Activity::Stats(_) => match no_activity {
            "7d" => "No activity in the last 7 days",
            _ => "No activity in the last 4 weeks",
        },
    }
}

/// One provider's activity: requests and sessions for 24 h and 7 days, and requests per day over the last week. The very
/// same layout and numbers for every provider, so they can be compared at a glance.
pub fn draw_stats_panel(c: &mut Canvas, top: i32, usage: &Usage) {
    let theme = theme_of(usage);
    draw_panel_header(c, top, usage, &FONTS);
    match &usage.activity {
        Activity::Stats(stats) => {
            for (i, label) in ["24h", "7d"].into_iter().enumerate() {
                let text = activity_stats::counts_text(label, stats.counts(label));
                c.text(10.0, f64::from(top + 28 + i as i32 * 15), &text, FONTS.body, TEXT);
            }
            draw_week_chart(c, &stats.days, 10, 230, top + 106, 34, theme.body, &FONTS);
        }
        other => c.text(10.0, f64::from(top + 40), nothing_to_show(other, "7d"), FONTS.body, MUTED),
    }
    dim_stale_panel(c, top, usage, &FONTS);
}

/// Where this week's requests went: the top projects on the left, the top models on the right (shares of the provider's
/// own total, same layout for every provider).
pub fn draw_breakdown_panel(c: &mut Canvas, top: i32, usage: &Usage) {
    let theme = theme_of(usage);
    draw_panel_header(c, top, usage, &FONTS);
    match &usage.activity {
        Activity::Stats(stats) if stats.total > 0 => {
            c.text(10.0, f64::from(top + 28), "Projects · 7d", FONTS.tiny, MUTED);
            draw_share_rows(c, &stats.projects, stats.total, 10, 104, top + 42, theme.body, &FONTS);
            c.text(126.0, f64::from(top + 28), "Models · 7d", FONTS.tiny, MUTED);
            draw_share_rows(c, &stats.models, stats.total, 126, 104, top + 42, theme.weekly, &FONTS);
        }
        other => c.text(10.0, f64::from(top + 40), nothing_to_show(other, "7d"), FONTS.body, MUTED),
    }
    dim_stale_panel(c, top, usage, &FONTS);
}

/// When you work: requests for each hour of the day over the last four weeks, with the busiest stretch called out.
pub fn draw_hours_panel(c: &mut Canvas, top: i32, usage: &Usage) {
    let theme = theme_of(usage);
    draw_panel_header(c, top, usage, &FONTS);
    let busy = match &usage.activity {
        Activity::Stats(stats) if stats.hours.iter().any(|h| *h > 0) => Some(stats.hours),
        _ => None,
    };
    match busy {
        None => c.text(10.0, f64::from(top + 40), nothing_to_show(&usage.activity, "4w"), FONTS.body, MUTED),
        Some(hours) => {
            let window = activity_stats::busiest_hours(&hours, 2).unwrap_or((0, 2));
            let text = format!("Busiest {} · last {} days", activity_stats::hours_text(window), activity_stats::HOURS_DAYS);
            c.text(10.0, f64::from(top + 27), &text, FONTS.body, TEXT);
            let peak = hours.iter().copied().max().unwrap_or(1);
            let (baseline, height) = (top + 102, 46);
            let dim = blend(theme.body, BG, 0.45);
            for (hour, n) in hours.iter().enumerate() {
                let x = 12 + hour as i32 * 9;
                let h = if *n > 0 { 2.max(pyround(f64::from(height) * *n as f64 / peak as f64) as i32) } else { 2 };
                // (the stretch wraps past midnight)
                let inside = (window.0 <= hour && hour < window.0 + 2) || (window.1 < window.0 && hour < window.1);
                let fill = if inside && *n > 0 { theme.body } else if *n > 0 { dim } else { PILL_BG };
                c.rect(x, baseline - h, x + 6, baseline - 1, fill);
            }
            for (hour, label) in [(0, "12a"), (6, "6a"), (12, "12p"), (18, "6p")] {
                c.text(f64::from(12 + hour * 9), f64::from(baseline + 1), label, FONTS.tiny, MUTED);
            }
        }
    }
    dim_stale_panel(c, top, usage, &FONTS);
}

pub type Drawer = fn(&mut Canvas, i32, &Usage);

pub fn render_panel_view(panels: &[Usage], drawer: Drawer) -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT, BG);
    for (i, usage) in panels.iter().take(2).enumerate() {
        drawer(&mut c, i as i32 * (SPLIT_PANEL_H + 4), usage);
    }
    c.rect(10, SPLIT_PANEL_H, 230, SPLIT_PANEL_H + 1, PILL_BG);
    c
}
