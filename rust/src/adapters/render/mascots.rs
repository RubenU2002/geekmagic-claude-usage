//! Each provider's pixel-art mascot.

use super::canvas::Canvas;
use crate::core::model::Usage;
use crate::core::themes::{theme_for_title, Theme, INK};

pub const MASCOT_TOP: i32 = 10;

pub fn theme_for(usage_title: &str) -> &'static Theme {
    theme_for_title(usage_title)
}

pub fn theme_of(usage: &Usage) -> &'static Theme {
    theme_for_title(&usage.title)
}

/// Blocky pixel-art mascot, drawn cell by cell so it stays crisp at small sizes.
pub fn draw_mascot(canvas: &mut Canvas, x0: i32, y0: i32, cell: i32, theme: &Theme) {
    for (row, line) in theme.bitmap.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if ch == '0' {
                continue;
            }
            let color = if ch == '1' { theme.body } else { INK };
            let (x, y) = (x0 + col as i32 * cell, y0 + row as i32 * cell);
            canvas.rect(x, y, x + cell - 1, y + cell - 1, color);
        }
    }
}

/// Width in pixels of a mascot drawn with 3 px cells.
pub fn mascot_width(theme: &Theme) -> i32 {
    3 * theme.bitmap[0].len() as i32
}
