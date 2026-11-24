//! A small RGB canvas with just the drawing the screens need: rectangles, rounded rectangles, ellipses and text.
//! (Pillow's job in the Python version.) Shapes are not anti-aliased, like Pillow's; text is.

use std::sync::OnceLock;

use ab_glyph::{point, Font, FontRef, GlyphId, PxScale, ScaleFont};

use crate::core::themes::Rgb;

static FONT_BYTES: &[u8] = include_bytes!("../../../assets/Aileron-Regular.ttf"); // Pillow's own default font (CC0)

fn font() -> &'static FontRef<'static> {
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| FontRef::try_from_slice(FONT_BYTES).expect("the bundled font is valid"))
}

/// `size` in pixels per em (what `ImageFont.load_default(size=N)` means), as the scale ab_glyph wants.
fn scale_for(size: f32) -> PxScale {
    let f = font();
    PxScale::from(size * f.height_unscaled() / f.units_per_em().unwrap_or(1000.0))
}

struct Laid {
    id: GlyphId,
    x: f32,
}

/// Glyphs with their pen positions (advances and kerning rounded to whole pixels, as a hinted rasteriser does), and the
/// total advance.
fn layout(text: &str, size: f32) -> (Vec<Laid>, f32) {
    let scaled = font().as_scaled(scale_for(size));
    let mut out = Vec::new();
    let mut pen = 0.0f32;
    let mut prev: Option<GlyphId> = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(p) = prev {
            pen += scaled.kern(p, id).round();
        }
        out.push(Laid { id, x: pen });
        pen += scaled.h_advance(id).round();
        prev = Some(id);
    }
    (out, pen)
}

/// Width in pixels of `text` at `size`.
pub fn text_width(text: &str, size: f32) -> f64 {
    f64::from(layout(text, size).1)
}

/// Pixels of ascent: where the baseline is below the top of a line of text.
fn ascent(size: f32) -> f32 {
    font().as_scaled(scale_for(size)).ascent().ceil()
}

/// (left, top, right, bottom) of `text` drawn at the origin, like `ImageDraw.textbbox((0, 0), ...)`.
pub fn text_bbox(text: &str, size: f32) -> (i32, i32, i32, i32) {
    let scale = scale_for(size);
    let asc = ascent(size);
    let (glyphs, advance) = layout(text, size);
    let (mut l, mut t, mut r, mut b) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for g in &glyphs {
        let glyph = g.id.with_scale_and_position(scale, point(g.x, asc));
        if let Some(outline) = font().outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            l = l.min(bounds.min.x.floor() as i32);
            r = r.max(bounds.max.x.ceil() as i32);
            t = t.min(bounds.min.y.floor() as i32);
            b = b.max(bounds.max.y.ceil() as i32);
        }
    }
    if l == i32::MAX {
        return (0, 0, advance.ceil() as i32, 0);
    }
    (l.max(0), t.max(0), r.max(advance.ceil() as i32), b)
}

#[derive(Clone)]
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    /// RGB, row by row.
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, bg: Rgb) -> Self {
        let mut px = Vec::with_capacity(w * h * 3);
        for _ in 0..w * h {
            px.extend_from_slice(&[bg.0, bg.1, bg.2]);
        }
        Self { w, h, px }
    }

    pub fn put(&mut self, x: i32, y: i32, c: Rgb) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 3;
        self.px[i] = c.0;
        self.px[i + 1] = c.1;
        self.px[i + 2] = c.2;
    }

    pub fn get(&self, x: usize, y: usize) -> Rgb {
        let i = (y * self.w + x) * 3;
        Rgb(self.px[i], self.px[i + 1], self.px[i + 2])
    }

    /// Filled rectangle, both corners included (like Pillow's `rectangle`).
    pub fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb) {
        for y in y0.max(0)..=y1.min(self.h as i32 - 1) {
            for x in x0.max(0)..=x1.min(self.w as i32 - 1) {
                self.put(x, y, c);
            }
        }
    }

    /// Filled rectangle with rounded corners, both corners included.
    pub fn rounded_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, radius: i32, c: Rgb) {
        let (w, h) = ((x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64);
        let r = f64::from(radius).min(w / 2.0).min(h / 2.0).max(0.0);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let nx = px.clamp(f64::from(x0) + r, f64::from(x1) + 1.0 - r);
                let ny = py.clamp(f64::from(y0) + r, f64::from(y1) + 1.0 - r);
                if (px - nx).powi(2) + (py - ny).powi(2) <= r * r + 1e-9 {
                    self.put(x, y, c);
                }
            }
        }
    }

    /// Filled ellipse inside the box, both corners included.
    pub fn ellipse(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb) {
        let (rx, ry) = (f64::from(x1 - x0 + 1) / 2.0, f64::from(y1 - y0 + 1) / 2.0);
        let (cx, cy) = (f64::from(x0) + rx, f64::from(y0) + ry);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = ((f64::from(x) + 0.5 - cx) / rx, (f64::from(y) + 0.5 - cy) / ry);
                if dx * dx + dy * dy <= 1.0 {
                    self.put(x, y, c);
                }
            }
        }
    }

    /// Mix the box (`x0..x1`, `y0..y1`, end excluded) towards `over`: 0 leaves it alone, 1 paints it `over`. Truncates
    /// like Pillow's `Image.blend`.
    pub fn blend_box(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, over: Rgb, alpha: f64) {
        for y in y0..y1.min(self.h) {
            for x in x0..x1.min(self.w) {
                let i = (y * self.w + x) * 3;
                for (k, o) in [over.0, over.1, over.2].into_iter().enumerate() {
                    let a = i32::from(self.px[i + k]);
                    // (single precision, like Pillow's C code: 100 -> 0 at 0.55 gives 45, not 44)
                    self.px[i + k] = (a as f32 + alpha as f32 * (i32::from(o) - a) as f32).clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    pub fn crop(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> Canvas {
        let (w, h) = (x1.min(self.w) - x0, y1.min(self.h) - y0);
        let mut px = Vec::with_capacity(w * h * 3);
        for y in y0..y0 + h {
            px.extend_from_slice(&self.px[(y * self.w + x0) * 3..(y * self.w + x0 + w) * 3]);
        }
        Canvas { w, h, px }
    }

    pub fn paste(&mut self, other: &Canvas, x: i32, y: i32) {
        for oy in 0..other.h {
            for ox in 0..other.w {
                self.put(x + ox as i32, y + oy as i32, other.get(ox, oy));
            }
        }
    }

    /// Draw `text` with its top-left at (x, y) (the top is the ascender line, like Pillow's default anchor).
    pub fn text(&mut self, x: f64, y: f64, text: &str, size: f32, c: Rgb) {
        let scale = scale_for(size);
        let baseline = y as f32 + ascent(size);
        let (glyphs, _) = layout(text, size);
        for g in &glyphs {
            let glyph = g.id.with_scale_and_position(scale, point(x as f32 + g.x, baseline));
            let Some(outline) = font().outline_glyph(glyph) else { continue };
            let bounds = outline.px_bounds();
            let (bx, by) = (bounds.min.x.floor() as i32, bounds.min.y.floor() as i32);
            outline.draw(|gx, gy, coverage| {
                let (px, py) = (bx + gx as i32, by + gy as i32);
                if coverage <= 0.0 || px < 0 || py < 0 || px as usize >= self.w || py as usize >= self.h {
                    return;
                }
                let i = (py as usize * self.w + px as usize) * 3;
                for (k, f) in [c.0, c.1, c.2].into_iter().enumerate() {
                    let bg = f32::from(self.px[i + k]);
                    self.px[i + k] = (bg + (f32::from(f) - bg) * coverage.min(1.0)).round() as u8;
                }
            });
        }
    }

    /// `text` shortened with "..." to fit `width` pixels.
    pub fn fit(text: &str, size: f32, width: f64) -> String {
        if text_width(text, size) <= width {
            return text.to_string();
        }
        let mut t: String = text.to_string();
        while !t.is_empty() && text_width(&format!("{t}..."), size) > width {
            t.pop();
        }
        format!("{}...", t.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: Rgb = Rgb(0, 0, 0);
    const WHITE: Rgb = Rgb(255, 255, 255);

    #[test]
    fn shapes_are_inclusive_and_clipped() {
        let mut c = Canvas::new(10, 10, BG);
        c.rect(2, 2, 4, 3, WHITE);
        assert_eq!(c.get(2, 2), WHITE);
        assert_eq!(c.get(4, 3), WHITE);
        assert_eq!(c.get(5, 3), BG);
        c.rect(-5, -5, 100, 0, WHITE); // clipped, no panic
        assert_eq!(c.get(9, 0), WHITE);
    }

    #[test]
    fn rounded_rect_cuts_the_corners_only() {
        let mut c = Canvas::new(40, 40, BG);
        c.rounded_rect(5, 5, 34, 34, 10, WHITE);
        assert_eq!(c.get(5, 5), BG); // corner pixel is outside the arc
        assert_eq!(c.get(20, 5), WHITE); // the straight edge is filled
        assert_eq!(c.get(20, 20), WHITE);
    }

    #[test]
    fn ellipse_fills_the_middle() {
        let mut c = Canvas::new(20, 20, BG);
        c.ellipse(2, 2, 12, 12, WHITE);
        assert_eq!(c.get(7, 7), WHITE);
        assert_eq!(c.get(2, 2), BG);
    }

    #[test]
    fn text_has_width_and_ink() {
        assert!(text_width("100%", 30.0) > text_width("1%", 30.0));
        assert_eq!(text_width("", 14.0), 0.0);
        let mut c = Canvas::new(120, 40, BG);
        c.text(2.0, 2.0, "Session", 14.0, WHITE);
        assert!(c.px.iter().any(|v| *v > 0), "something was drawn");
        let (l, _, r, b) = text_bbox("Session", 14.0);
        assert!(r - l > 20 && b > 5);
    }

    #[test]
    fn fit_shortens_with_dots() {
        assert_eq!(Canvas::fit("hi", 10.0, 100.0), "hi");
        let fitted = Canvas::fit("a very long project name indeed", 10.0, 60.0);
        assert!(fitted.ends_with("...") && text_width(&fitted, 10.0) <= 60.0);
    }

    #[test]
    fn blend_truncates_like_pillow() {
        let mut c = Canvas::new(2, 2, Rgb(100, 100, 100));
        c.blend_box(0, 0, 2, 2, Rgb(0, 0, 0), 0.55);
        assert_eq!(c.get(0, 0), Rgb(45, 45, 45)); // checked against Pillow's Image.blend
        let mut d = Canvas::new(1, 1, Rgb(250, 84, 7));
        d.blend_box(0, 0, 1, 1, Rgb(24, 22, 15), 0.55);
        assert_eq!(d.get(0, 0), Rgb(125, 49, 11)); // ...and so is this one (truncated, not rounded)
    }
}
