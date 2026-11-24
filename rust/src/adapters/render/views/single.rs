//! One provider's screen: mascot, two bars, reset countdowns, pace.

use super::super::animations::{animation, draw_layer, Layer};
use super::super::canvas::{text_bbox, text_width, Canvas};
use super::super::components::*;
use super::super::gif::encode_gif;
use super::super::mascots::{draw_mascot, mascot_width, theme_of, MASCOT_TOP};
use super::super::palette::*;
use crate::core::model::{Usage, WindowKind};

const TITLE_FONT: f32 = 26.0;

pub fn render_frame(usage: &Usage, mascot_dx: i32, mascot_dy: i32, extra: Option<&Layer>) -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT, BG);
    let theme = theme_of(usage);
    let mascot_w = mascot_width(theme);
    draw_mascot(&mut c, 10 + mascot_dx, MASCOT_TOP + mascot_dy, 3, theme);
    if let Some(layer) = extra {
        draw_layer(&mut c, layer, mascot_w);
    }

    c.text(f64::from(10 + mascot_w + 10), 8.0, &usage.title, TITLE_FONT, TEXT);
    let busy = busy_style(usage); // its agent is running (green "Working") or waiting for you (amber "Waiting"), top right
    if let Some((label, color)) = busy {
        let label_w = text_width(label, FOOTER_FONT);
        c.text(230.0 - label_w, 7.0, label, FOOTER_FONT, color);
        draw_working_dot(&mut c, 230.0 - label_w - 11.0, 15.0, color);
    }
    if let Some((text, color)) = resets_chip(usage) {
        // Codex only: the free rate-limit resets you still have, top right
        c.text(230.0 - text_width(&text, FOOTER_FONT), if busy.is_some() { 23.0 } else { 15.0 }, &text, FOOTER_FONT, color);
    }

    for (top, label, kind, accent) in [
        (40, "Session", WindowKind::Current, theme.current),
        (134, "Weekly", WindowKind::Weekly, theme.weekly),
    ] {
        let window = usage.window(kind);
        draw_section(
            &mut c,
            &Section {
                top,
                label,
                percent: window.pct,
                reset_text: format_delta(window.reset, usage.now),
                accent,
                pace_info: window_pace(usage, kind),
            },
        );
    }

    let (footer, footer_color) = if usage.stale {
        // Last known numbers that couldn't be refreshed: dim everything and say since when.
        c.blend_box(0, 0, WIDTH, HEIGHT, BG, 0.55);
        (format!("! No fresh data since {}", clock(usage.now)), WARN)
    } else {
        (format!("* Updated {}", clock(usage.now)), theme.current)
    };
    let bbox = text_bbox(&footer, FOOTER_FONT);
    c.text(f64::from((240 - (bbox.2 - bbox.0)) / 2), 223.0, &footer, FOOTER_FONT, footer_color);
    c
}

/// Looping GIF for one of the named animations.
pub fn render_animation(usage: &Usage, name: &str) -> Result<Vec<u8>, String> {
    let frames = animation(name).ok_or_else(|| format!("no animation called {name}"))?;
    let canvases: Vec<Canvas> = frames.iter().map(|f| render_frame(usage, f.dx, f.dy, f.extra.as_ref())).collect();
    encode_gif(&canvases)
}
