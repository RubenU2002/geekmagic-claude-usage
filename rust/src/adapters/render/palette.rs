//! Colours and sizes shared by every screen.

use crate::core::alerts::BarState;
use crate::core::themes::{hex, Rgb};

pub use crate::core::themes::{blend, BG};

pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 240;
/// Two panels of this height, a 4 px divider between them.
pub const SPLIT_PANEL_H: i32 = 118;

pub const PILL_BG: Rgb = hex("#4A3F4D");
/// Past the thresholds in `core::alerts` the percentage and its bar swap the provider's accent for these.
pub const WARN: Rgb = hex("#FFD23F");
pub const CRIT: Rgb = hex("#FF2D55");
pub const TEXT: Rgb = hex("#F5F4EF");
pub const MUTED: Rgb = hex("#C7C2BC");
/// The dot (and word) that says an agent is running right now.
pub const WORKING_COLOR: Rgb = hex("#4ADE80");
/// ...and amber when it has stopped and is waiting for you.
pub const WAITING_COLOR: Rgb = hex("#FFB020");

/// The colour a bar state asks for, if it asks for one (`Ok` keeps the provider's own).
pub fn state_color(state: BarState) -> Option<Rgb> {
    match state {
        BarState::Ok => None,
        BarState::Warn => Some(WARN),
        BarState::Crit => Some(CRIT),
    }
}
