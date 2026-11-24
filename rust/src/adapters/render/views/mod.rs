//! The screens, one function each. A view turns the readings it is given into the GIF to upload, and says which
//! rotation picks it made, so the caller can remember them once the upload has gone through.

pub mod error;
pub mod panels;
pub mod single;
pub mod split;

use super::animations::{animation_for, busy_animation, Rotation};
use super::gif::encode_gif;
use crate::core::model::{ErrorScreen, Usage};
use crate::core::ports::Rendered;

fn rotating(animation: &str) -> bool {
    animation == "auto" || animation == "random"
}

/// One provider's screen, with its mascot playing an animation.
pub fn render_single(usage: &Usage, animation: &str, rotation: &Rotation) -> Result<Rendered, String> {
    let mut animation = animation.to_string();
    let mut pick = rotating(&animation);
    if pick {
        if let Some(busy) = busy_animation(usage) {
            // its agent is busy: the mascot shows it (and that isn't a rotation pick)
            animation = busy.to_string();
            pick = false;
        } else {
            animation = rotation.pick(&usage.title).to_string();
        }
    }
    let gif = single::render_animation(usage, &animation)?;
    let picks = if pick { vec![(usage.title.clone(), animation.clone())] } else { Vec::new() };
    Ok(Rendered { gif, animations: vec![animation], picks })
}

/// A provider that can't be read: its mascot and why.
pub fn render_error(screen: &ErrorScreen) -> Result<Rendered, String> {
    Ok(Rendered { gif: encode_gif(&[error::render_error_frame(screen)])?, ..Default::default() })
}

/// Both providers on one screen: each gets a fresh animation of its own ("auto" / "random"), remembered once uploaded.
pub fn render_split_view(panels: &[Usage], animation: &str, rotation: &Rotation) -> Result<Rendered, String> {
    let names: Vec<String> = panels.iter().take(2).map(|u| animation_for(u, animation, rotation)).collect();
    let picks = if rotating(animation) {
        // busy agent: its mascot works, which isn't a rotation pick
        panels.iter().take(2).zip(&names).filter(|(u, _)| !u.working).map(|(u, n)| (u.title.clone(), n.clone())).collect()
    } else {
        Vec::new()
    };
    Ok(Rendered { gif: split::render_split(panels, &names)?, animations: names, picks })
}

/// The views of both providers' activity: a single still frame (a small upload).
pub fn render_still_view(key: &str, panels: &[Usage]) -> Result<Rendered, String> {
    let drawer: panels::Drawer = match key {
        crate::core::views::STATS => panels::draw_stats_panel,
        crate::core::views::BREAKDOWN => panels::draw_breakdown_panel,
        crate::core::views::HOURS => panels::draw_hours_panel,
        other => return Err(format!("no view draws {other}")),
    };
    Ok(Rendered { gif: encode_gif(&[panels::render_panel_view(panels, drawer)])?, ..Default::default() })
}
