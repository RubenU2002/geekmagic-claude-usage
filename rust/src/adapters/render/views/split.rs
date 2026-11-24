//! The split view: two providers on one screen, each mascot doing its own animation.

use super::super::animations::{animation, draw_layer, Frame, IDLE_BOB};
use super::super::canvas::{text_width, Canvas};
use super::super::components::*;
use super::super::gif::encode_gif;
use super::super::mascots::{draw_mascot, mascot_width, theme_of, MASCOT_TOP};
use super::super::palette::*;
use crate::core::model::{Usage, WindowKind};

/// The mascot-and-props corner of a split panel: the same art, at the same scale, as the single screens.
const SPLIT_ART: (usize, usize) = (64, 40);

pub const FONTS: Fonts = Fonts { title: 18.0, body: 12.0, small: 11.0, tiny: 10.0, pct: 22.0 };

/// The top-left corner of a single-provider screen for one animation frame (mascot at full size plus whatever it holds).
/// The animations draw at absolute coordinates, so draw the real thing on a scratch screen and cut the corner out.
fn header_art(usage: &Usage, frame: Option<&Frame>) -> Canvas {
    let mut scratch = Canvas::new(WIDTH, HEIGHT, BG);
    let theme = theme_of(usage);
    let (dx, dy) = frame.map_or((0, 0), |f| (f.dx, f.dy));
    draw_mascot(&mut scratch, 10 + dx, MASCOT_TOP + dy, 3, theme);
    if let Some(layer) = frame.and_then(|f| f.extra.as_ref()) {
        draw_layer(&mut scratch, layer, mascot_width(theme));
    }
    scratch.crop(0, 0, SPLIT_ART.0, SPLIT_ART.1)
}

/// One provider's panel (animated header + two rows) at vertical offset `top`; dimmed and dated if stale.
fn draw_split_panel(c: &mut Canvas, top: i32, usage: &Usage, frame: &Frame) {
    let theme = theme_of(usage);
    c.paste(&header_art(usage, if usage.stale { None } else { Some(frame) }), 0, top);
    c.text(SPLIT_ART.0 as f64 + 2.0, f64::from(top + 8), &usage.title, FONTS.title, TEXT);
    let updated = clock(usage.now);
    if !usage.stale {
        let width = text_width(&updated, FONTS.small);
        c.text(230.0 - width, f64::from(top + 12), &updated, FONTS.small, theme.current);
        draw_busy_tag(c, f64::from(top + 25), usage, FONTS.small, None);
    }
    for (row_top, label, kind, accent) in [
        (top + 40, "Session", WindowKind::Current, theme.current),
        (top + 77, "Weekly", WindowKind::Weekly, theme.weekly),
    ] {
        let window = usage.window(kind);
        draw_split_row(c, row_top, label, window.pct, &format_delta(window.reset, usage.now), accent, &FONTS, window_pace(usage, kind).as_ref());
    }
    if usage.stale {
        c.blend_box(0, top as usize, WIDTH, (top + SPLIT_PANEL_H) as usize, BG, 0.55);
        let text = format!("! stale since {updated}");
        c.text(230.0 - text_width(&text, FONTS.small), f64::from(top + 12), &text, FONTS.small, WARN);
    }
}

fn frames_of(names: &[String], i: usize) -> Result<&'static [Frame], String> {
    let name = names.get(i).map_or("idle", String::as_str);
    animation(name).ok_or_else(|| format!("no animation called {name}"))
}

/// Frame `index` of the split view. Each panel plays its own animation; a shorter one simply loops again while a longer
/// one is still going.
pub fn render_split_frame(panels: &[Usage], animations: &[String], index: usize) -> Result<Canvas, String> {
    let mut c = Canvas::new(WIDTH, HEIGHT, BG);
    for (i, usage) in panels.iter().take(2).enumerate() {
        let spec = frames_of(animations, i)?;
        draw_split_panel(&mut c, i as i32 * (SPLIT_PANEL_H + 4), usage, &spec[index % spec.len()]);
    }
    c.rect(10, SPLIT_PANEL_H, 230, SPLIT_PANEL_H + 1, PILL_BG); // divider
    Ok(c)
}

/// Looping GIF with two providers' usage stacked on one screen, each mascot doing its own animation.
pub fn render_split(panels: &[Usage], animations: &[String]) -> Result<Vec<u8>, String> {
    let mut length = IDLE_BOB.len();
    if !panels.is_empty() {
        length = 0;
        for i in 0..panels.len().min(2) {
            length = length.max(frames_of(animations, i)?.len());
        }
    }
    let frames: Result<Vec<Canvas>, String> = (0..length).map(|index| render_split_frame(panels, animations, index)).collect();
    encode_gif(&frames?)
}
