//! How each provider looks: its mascot, its colours. Pure data, shared by everything that draws something (the screens,
//! the tray icon).

use crate::util::pyround;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

const fn hv(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

/// "#RRGGBB" as a colour, usable in constants.
pub const fn hex(s: &str) -> Rgb {
    let b = s.as_bytes();
    Rgb(hv(b[1]) * 16 + hv(b[2]), hv(b[3]) * 16 + hv(b[4]), hv(b[5]) * 16 + hv(b[6]))
}

/// `color` mixed with `over` (0 = all `over`, 1 = all `color`).
pub fn blend(color: Rgb, over: Rgb, amount: f64) -> Rgb {
    let mix = |a: u8, b: u8| pyround(f64::from(b) + (f64::from(a) - f64::from(b)) * amount).clamp(0, 255) as u8;
    Rgb(mix(color.0, over.0), mix(color.1, over.1), mix(color.2, over.2))
}

/// The screens' background: warm ink, laid out like a watch face.
pub const BG: Rgb = hex("#18160F");
pub const CURRENT_ACCENT: Rgb = hex("#FA5407"); // Claude's vivid, saturated orange
pub const WEEKLY_ACCENT: Rgb = hex("#E8B24D");
pub const ACCENT: Rgb = CURRENT_ACCENT; // used by the mascot's body
pub const INK: Rgb = hex("#0F0D0B"); // the mascot's eyes and the dark parts of what it holds

/// Pixel-art mascot bitmaps, 14 columns x 8 rows: '1' = body, '2' = eye ink, '0' = background.
pub type Bitmap = [&'static str; 8];

pub const CLAUDE_BITMAP: Bitmap = [
    "00011111111000",
    "00011211211000",
    "01111111111110",
    "01111111111110",
    "00011111111000",
    "00011111111000",
    "00010100101000",
    "00010100101000",
];

/// Codex's mascot: a little cloud, same 14x8 footprint as the Claude one. Its `>_` prompt is drawn per frame.
pub const CODEX_BITMAP: Bitmap = [
    "00000000000000",
    "00000011110000",
    "00011011111100",
    "00111111111110",
    "01111111111111",
    "01111111111111",
    "01111111111111",
    "00111111111110",
];

#[derive(Debug)]
pub struct Theme {
    pub bitmap: &'static Bitmap,
    pub body: Rgb,
    pub current: Rgb,
    pub weekly: Rgb,
}

pub static CLAUDE: Theme = Theme { bitmap: &CLAUDE_BITMAP, body: ACCENT, current: CURRENT_ACCENT, weekly: WEEKLY_ACCENT };
pub static CODEX: Theme = Theme { bitmap: &CODEX_BITMAP, body: hex("#4F9DFF"), current: hex("#4F9DFF"), weekly: hex("#B79CFF") };

/// The look of a provider, picked by its title (anything unknown looks like Claude).
pub fn theme_for_title(title: &str) -> &'static Theme {
    if title == "Codex" {
        &CODEX
    } else {
        &CLAUDE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_blend() {
        assert_eq!(hex("#FA5407"), Rgb(250, 84, 7));
        assert_eq!(blend(Rgb(200, 100, 0), Rgb(0, 0, 0), 0.5), Rgb(100, 50, 0));
        assert_eq!(blend(Rgb(255, 255, 255), Rgb(0, 0, 0), 0.45), Rgb(115, 115, 115));
    }
}
