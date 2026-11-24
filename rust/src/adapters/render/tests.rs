//! Smoke tests of every screen: each renders, stays small, and decodes. Set `RENDER_DUMP=<dir>` to also write each one
//! as a PNG to look at.

use std::sync::Arc;

use chrono::{Duration, Local, TimeZone, Utc};

use super::*;
use crate::core::activity::{Counts, DayCount, Named, Summary};
use crate::core::model::{Activity, CodexExtra, Data, ErrorScreen, Usage, Window};
use crate::core::pace::annotate;
use crate::core::views::{BREAKDOWN, HOURS, SPLIT, STATS};

fn now() -> chrono::DateTime<Local> {
    Local.with_ymd_and_hms(2026, 10, 2, 14, 57, 0).unwrap()
}

fn usage(title: &str, session: f64, week: f64) -> Usage {
    let n = now();
    let mut u = Usage::new(
        title,
        Window { pct: Some(session), reset: Some(n.with_timezone(&Utc) + Duration::minutes(90)), minutes: None },
        Window { pct: Some(week), reset: Some(n.with_timezone(&Utc) + Duration::hours(52)), minutes: None },
        n,
    );
    annotate(&mut u, [None, None]);
    u
}

fn summary() -> Arc<Summary> {
    let mut hours = [0usize; 24];
    for (h, n) in [(9, 12), (10, 30), (11, 41), (14, 25), (15, 38), (16, 9), (22, 3)] {
        hours[h] = n;
    }
    Arc::new(Summary {
        h24: Counts { requests: 375, sessions: 4 },
        d7: Counts { requests: 2210, sessions: 19 },
        days: ["2026-09-26", "2026-09-27", "2026-09-28", "2026-09-29", "2026-09-30", "2026-10-01", "2026-10-02"]
            .iter()
            .zip([120, 0, 310, 505, 640, 260, 375])
            .map(|(d, n)| DayCount { date: (*d).into(), requests: n })
            .collect(),
        projects: vec![
            Named { name: "Acme App".into(), requests: 900 },
            Named { name: "Storefront/frontend".into(), requests: 700 },
            Named { name: "a-very-long-project-name-here".into(), requests: 400 },
        ],
        models: vec![Named { name: "Opus 5.5".into(), requests: 1500 }, Named { name: "Sonnet 5".into(), requests: 700 }],
        total: 2210,
        hours,
    })
}

fn dump(name: &str, gif: &[u8]) {
    if let Ok(dir) = std::env::var("RENDER_DUMP") {
        let png = SoftRenderer::new().to_png(gif).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), png).unwrap();
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.gif")), gif).unwrap();
    }
}

fn frame_count(gif: &[u8]) -> usize {
    let mut decoder = ::gif::DecodeOptions::new().read_info(gif).unwrap();
    let mut n = 0;
    while decoder.read_next_frame().unwrap().is_some() {
        n += 1;
    }
    n
}

#[test]
fn every_single_animation_renders_small_and_loops() {
    let r = SoftRenderer::new();
    for (title, name) in [("Claude", "laptop"), ("Claude", "coffee"), ("Claude", "typing"), ("Claude", "dance"), ("Claude", "eureka"),
                          ("Codex", "prompt"), ("Codex", "bolt"), ("Codex", "hop"), ("Codex", "sparkle"), ("Codex", "code")] {
        let mut u = usage(title, 62.0, 35.0);
        if title == "Codex" {
            u.codex_extra = Some(CodexExtra { free_resets: 2, next_reset_expires_days: Some(2) });
        }
        let out = r.render(&title.to_lowercase(), &Data::One(u), name).unwrap();
        assert_eq!(out.animations, [name]);
        assert!(out.picks.is_empty(), "a named animation is not a rotation pick");
        assert!(out.gif.len() < 60_000, "{title}/{name}: {} bytes", out.gif.len());
        assert_eq!(frame_count(&out.gif), animations::animation(name).unwrap().len());
        dump(&format!("single-{title}-{name}"), &out.gif);
    }
}

#[test]
fn auto_picks_and_remembers_only_after_confirm() {
    let r = SoftRenderer::new();
    let out = r.render("claude", &Data::One(usage("Claude", 10.0, 5.0)), "auto").unwrap();
    assert_eq!(out.picks.len(), 1);
    assert!(r.snapshot()["animations"].as_object().unwrap().is_empty(), "not remembered before it is on the device");
    r.confirm(&out);
    assert_eq!(r.snapshot()["animations"]["Claude"][0], out.animations[0].as_str());
}

#[test]
fn a_busy_agent_makes_its_mascot_work_without_a_pick() {
    let r = SoftRenderer::new();
    let mut u = usage("Claude", 10.0, 5.0);
    u.working = true;
    let out = r.render("claude", &Data::One(u.clone()), "auto").unwrap();
    assert_eq!((out.animations[0].as_str(), out.picks.len()), ("typing", 0));
    u.waiting = true;
    dump("single-waiting", &r.render("claude", &Data::One(u), "auto").unwrap().gif);
}

#[test]
fn warn_crit_and_stale_screens_render() {
    let r = SoftRenderer::new();
    let mut hot = usage("Claude", 96.0, 74.0);
    hot.pace = [None, None];
    let out = r.render("claude", &Data::One(hot.clone()), "idle").unwrap();
    dump("single-hot", &out.gif);
    hot.stale = true;
    dump("single-stale", &r.render("claude", &Data::One(hot), "idle").unwrap().gif);
    let missing = Usage::new("Claude", Window::default(), Window::default(), now());
    dump("single-dashes", &r.render("claude", &Data::One(missing), "idle").unwrap().gif);
}

#[test]
fn the_views_of_two_render() {
    let r = SoftRenderer::new();
    let mut claude = usage("Claude", 62.0, 35.0);
    let mut codex = usage("Codex", 91.0, 12.0);
    codex.working = true;
    let split = r.render(SPLIT, &Data::Panels(vec![claude.clone(), codex.clone()]), "auto").unwrap();
    assert_eq!(split.animations.len(), 2);
    assert!(split.gif.len() < 60_000, "{} bytes", split.gif.len());
    dump("split", &split.gif);
    claude.activity = Activity::Stats(summary());
    codex.activity = Activity::NoLogs;
    for key in [STATS, BREAKDOWN, HOURS] {
        let out = r.render(key, &Data::Panels(vec![claude.clone(), codex.clone()]), "auto").unwrap();
        assert_eq!(frame_count(&out.gif), 1, "{key} is a single still frame");
        dump(key, &out.gif);
    }
    claude.activity = Activity::Counting;
    claude.stale = true;
    dump("stats-counting-stale", &r.render(STATS, &Data::Panels(vec![claude, codex]), "auto").unwrap().gif);
}

#[test]
fn the_error_screen_wraps_its_message() {
    let r = SoftRenderer::new();
    let screen = ErrorScreen {
        title: "Codex".into(),
        message: "A window opened: finish signing in to Codex there. This screen fills in by itself.".into(),
        now: now(),
        signin: true,
    };
    let out = r.render("codex", &Data::Error(screen), "idle").unwrap();
    assert_eq!(frame_count(&out.gif), 1);
    dump("error", &out.gif);
}

#[test]
fn to_png_is_a_png_of_the_first_frame() {
    let r = SoftRenderer::new();
    let out = r.render("claude", &Data::One(usage("Claude", 62.0, 35.0)), "laptop").unwrap();
    let png = r.to_png(&out.gif).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(r.to_png(b"not a gif").is_err());
}

#[test]
fn render_id_is_set() {
    assert_eq!(RENDER_ID.len(), 12);
}
