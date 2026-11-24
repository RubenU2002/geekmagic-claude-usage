//! The little things a mascot holds: a laptop, a mug of coffee, an idea spark.

use super::canvas::Canvas;
use crate::core::themes::{hex, Rgb};

/// A tiny pixel-art laptop: 'B' = bezel/base, 'G' = the glowing screen, '.' = background.
pub const LAPTOP_BITMAP: [&str; 6] = ["BBBBBBBB", "BGGGGGGB", "BGGGGGGB", "BBBBBBBB", ".BBBBBB.", "BBBBBBBB"];
pub const LAPTOP_BEZEL: Rgb = hex("#4A3F4D");
pub const LAPTOP_GLOW: Rgb = hex("#FFE9C7");

pub fn draw_laptop(canvas: &mut Canvas, x0: i32, y0: i32, cell: i32) {
    for (row, line) in LAPTOP_BITMAP.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if ch == '.' {
                continue;
            }
            let color = if ch == 'B' { LAPTOP_BEZEL } else { LAPTOP_GLOW };
            let (x, y) = (x0 + col as i32 * cell, y0 + row as i32 * cell);
            canvas.rect(x, y, x + cell - 1, y + cell - 1, color);
        }
    }
}

/// (x, hidden_y, held_y) shared by the laptop and the typing-cursor overlay.
pub fn laptop_geometry(mascot_w: i32) -> (i32, i32, i32) {
    let laptop_w = 3 * LAPTOP_BITMAP[0].len() as i32;
    (10 + (mascot_w - laptop_w) / 2, 34, 21)
}

/// A tiny pixel-art mug: 'B' = ceramic, 'C' = coffee.
pub const MUG_BITMAP: [&str; 5] = ["BBBBBB", "BCCCCB", "BCCCCB", "BCCCCB", "BBBBBB"];
pub const MUG_CERAMIC: Rgb = hex("#F5F4EF");
pub const MUG_COFFEE: Rgb = hex("#5A3A22");
pub const STEAM_COLOR: Rgb = hex("#C7C2BC");

pub fn draw_mug(canvas: &mut Canvas, x0: i32, y0: i32, cell: i32, steam_phase: i32) {
    for (row, line) in MUG_BITMAP.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            let color = if ch == 'B' { MUG_CERAMIC } else { MUG_COFFEE };
            let (x, y) = (x0 + col as i32 * cell, y0 + row as i32 * cell);
            canvas.rect(x, y, x + cell - 1, y + cell - 1, color);
        }
    }
    // Two wisps of steam, each drifting up and fading over a cycle.
    for (wisp, col) in [1, 4].into_iter().enumerate() {
        let phase = (steam_phase + wisp as i32 * 2) % 6;
        if phase >= 4 {
            continue;
        }
        let (x, y) = (x0 + col * cell, y0 - (phase + 1) * cell);
        canvas.rect(x, y, x + cell - 1, y + cell - 1, STEAM_COLOR);
    }
}

/// A tiny "idea" spark for the "eureka" moment: a diamond burst.
pub const BULB_BITMAP: [&str; 5] = ["..G..", ".GGG.", "GGGGG", ".GGG.", "..G.."];
pub const BULB_GLOW_ON: Rgb = hex("#FFD966");
pub const BULB_GLOW_OFF: Rgb = hex("#6B5E3E");

pub fn draw_bulb(canvas: &mut Canvas, x0: i32, y0: i32, cell: i32, lit: bool) {
    let glow = if lit { BULB_GLOW_ON } else { BULB_GLOW_OFF };
    for (row, line) in BULB_BITMAP.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if ch == '.' {
                continue;
            }
            let (x, y) = (x0 + col as i32 * cell, y0 + row as i32 * cell);
            canvas.rect(x, y, x + cell - 1, y + cell - 1, glow);
        }
    }
}
