//! The tray icon: the mascot of the provider on screen, or a picture of the view of two that is.

use crate::core::themes::{blend, hex, theme_for_title, Rgb, BG, CLAUDE, CODEX, INK};
use crate::core::views::{BREAKDOWN, HOURS, SPLIT, STATS};

/// A small RGBA picture (transparent where nothing is drawn).
#[derive(Clone)]
pub struct Rgba {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Rgba {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, px: vec![0; w * h * 4] }
    }

    fn put(&mut self, x: i32, y: i32, c: Rgb) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 4;
        self.px[i..i + 4].copy_from_slice(&[c.0, c.1, c.2, 255]);
    }

    /// Filled rectangle, both corners included.
    pub fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.put(x, y, c);
            }
        }
    }

    /// Nearest-neighbour resize (the way Pillow's NEAREST picks pixels).
    pub fn resized(&self, w: usize, h: usize) -> Rgba {
        let mut out = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (((x as f64 + 0.5) * self.w as f64 / w as f64) as usize, ((y as f64 + 0.5) * self.h as f64 / h as f64) as usize);
                let (s, d) = ((sy.min(self.h - 1) * self.w + sx.min(self.w - 1)) * 4, (y * w + x) * 4);
                out.px[d..d + 4].copy_from_slice(&self.px[s..s + 4]);
            }
        }
        out
    }

    /// Copy `other` onto this picture with its top-left at (x, y).
    pub fn paste(&mut self, other: &Rgba, x: i32, y: i32) {
        for oy in 0..other.h {
            for ox in 0..other.w {
                let (tx, ty) = (x + ox as i32, y + oy as i32);
                if tx < 0 || ty < 0 || tx as usize >= self.w || ty as usize >= self.h {
                    continue;
                }
                let (s, d) = ((oy * other.w + ox) * 4, (ty as usize * self.w + tx as usize) * 4);
                self.px[d..d + 4].copy_from_slice(&other.px[s..s + 4]);
            }
        }
    }
}

fn mascot_image(title: &str) -> Rgba {
    let theme = theme_for_title(title);
    let mut image = Rgba::new(42, 24);
    for (row, line) in theme.bitmap.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if ch != '0' {
                let color = if ch == '1' { theme.body } else { hex("#0F0D0B") };
                let (x, y) = (col as i32 * 3, row as i32 * 3);
                image.rect(x, y, x + 2, y + 2, color);
            }
        }
    }
    let _ = INK;
    image
}

/// Mascot on a transparent background, tinted per provider so you can tell them apart.
pub fn make_icon(title: &str) -> Rgba {
    let mut canvas = Rgba::new(64, 64);
    canvas.paste(&mascot_image(title).resized(60, 34), 2, 15);
    canvas
}

/// Both mascots stacked: the icon shown while the split view is on.
pub fn make_split_icon() -> Rgba {
    let mut canvas = Rgba::new(64, 64);
    canvas.paste(&mascot_image("Claude"), 11, 4);
    canvas.paste(&mascot_image("Codex"), 11, 36);
    canvas
}

/// Three bars in the providers' colours: the icon shown while the stats view is on.
pub fn make_stats_icon() -> Rgba {
    let mut canvas = Rgba::new(64, 64);
    for (i, (height, color)) in [(26, CLAUDE.body), (40, CODEX.body), (18, CODEX.weekly)].into_iter().enumerate() {
        let x = 10 + i as i32 * 17;
        canvas.rect(x, 54 - height, x + 12, 54, color);
    }
    canvas
}

/// Three horizontal bars of different lengths: shares of a total.
pub fn make_breakdown_icon() -> Rgba {
    let mut canvas = Rgba::new(64, 64);
    for (i, (length, color)) in [(46, CLAUDE.body), (32, CODEX.body), (18, CODEX.weekly)].into_iter().enumerate() {
        let y = 12 + i as i32 * 15;
        canvas.rect(8, y, 8 + length, y + 9, color);
    }
    canvas
}

/// A little histogram with its busy stretch in the middle: the hours of the day.
pub fn make_hours_icon() -> Rgba {
    let mut canvas = Rgba::new(64, 64);
    for (i, height) in [8, 12, 22, 38, 44, 30, 16, 10].into_iter().enumerate() {
        let x = 6 + i as i32 * 7;
        let color = if (3..=4).contains(&i) { CLAUDE.body } else { blend(CLAUDE.body, BG, 0.45) };
        canvas.rect(x, 54 - height, x + 5, 54, color);
    }
    canvas
}

/// The icon of a view: a provider's key (`titles` says what it is called), or one of the views of two.
pub fn icon_for(key: &str, titles: &[(String, String)]) -> Rgba {
    match key {
        SPLIT => make_split_icon(),
        STATS => make_stats_icon(),
        BREAKDOWN => make_breakdown_icon(),
        HOURS => make_hours_icon(),
        _ => make_icon(titles.iter().find(|(k, _)| k == key).map_or("Claude", |(_, t)| t.as_str())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles() -> Vec<(String, String)> {
        vec![("claude".into(), "Claude".into()), ("codex".into(), "Codex".into())]
    }

    fn opaque(i: &Rgba) -> usize {
        i.px.chunks_exact(4).filter(|p| p[3] == 255).count()
    }

    #[test]
    fn every_view_has_its_own_icon_of_the_same_size() {
        let keys = ["claude", "codex", SPLIT, STATS, BREAKDOWN, HOURS];
        let icons: Vec<Rgba> = keys.iter().map(|k| icon_for(k, &titles())).collect();
        assert!(icons.iter().all(|i| (i.w, i.h) == (64, 64) && opaque(i) > 100));
        for (a, b) in [(0, 1), (0, 2), (2, 3), (3, 4), (4, 5)] {
            assert_ne!(icons[a].px, icons[b].px, "{} and {} look alike", keys[a], keys[b]);
        }
    }

    #[test]
    fn the_mascot_is_tinted_per_provider_on_a_transparent_background() {
        let claude = make_icon("Claude");
        assert_eq!(&claude.px[..4], &[0, 0, 0, 0], "transparent corner");
        let body = CLAUDE.body;
        let has = |i: &Rgba, c: Rgb| i.px.chunks_exact(4).any(|p| p[..3] == [c.0, c.1, c.2] && p[3] == 255);
        assert!(has(&claude, body) && !has(&claude, CODEX.body));
        assert!(has(&make_icon("Codex"), CODEX.body));
    }

    #[test]
    fn nearest_resize_keeps_hard_edges() {
        let mut small = Rgba::new(2, 1);
        small.rect(0, 0, 0, 0, Rgb(255, 0, 0));
        small.rect(1, 0, 1, 0, Rgb(0, 0, 255));
        let big = small.resized(4, 2);
        assert_eq!(&big.px[..4], &[255, 0, 0, 255]);
        assert_eq!(&big.px[3 * 4..4 * 4], &[0, 0, 255, 255]);
    }
}
