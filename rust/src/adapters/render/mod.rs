//! The Renderer port, drawn by hand on a small canvas: readings in, a GIF out.

pub mod animations;
pub mod canvas;
pub mod components;
pub mod gif;
pub mod mascots;
pub mod palette;
pub mod props;
pub mod views;

use serde_json::{Map, Value};

use crate::core::model::Data;
use crate::core::ports::{Rendered, Renderer};
use crate::core::views::{is_panel_key, SPLIT};
use animations::Rotation;

/// Identifies the drawing code (see build.rs): the device keeps the images it was sent, so after an update what is
/// stored is out of date and the app redraws every view once.
pub const RENDER_ID: &str = env!("RENDER_ID");

pub struct SoftRenderer {
    rotation: Rotation, // which animation each provider played lately, so none comes back too soon
}

impl SoftRenderer {
    pub fn new() -> Self {
        Self { rotation: Rotation::default() }
    }
}

impl Default for SoftRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for SoftRenderer {
    fn version(&self) -> &str {
        RENDER_ID
    }

    fn render(&self, key: &str, data: &Data, animation: &str) -> Result<Rendered, String> {
        match data {
            Data::Error(screen) => views::render_error(screen),
            Data::One(usage) => views::render_single(usage, animation, &self.rotation),
            Data::Panels(panels) if is_panel_key(key) => {
                if key == SPLIT {
                    views::render_split_view(panels, animation, &self.rotation)
                } else {
                    views::render_still_view(key, panels)
                }
            }
            Data::Panels(_) => Err(format!("{key} is not a view of two providers")),
        }
    }

    fn to_png(&self, gif_bytes: &[u8]) -> Result<Vec<u8>, String> {
        // the whole picture, the way the screen first shows it (the later frames only move the mascot)
        let (w, h, rgb) = gif::first_frame_rgb(gif_bytes)?;
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
        Ok(out)
    }

    fn confirm(&self, rendered: &Rendered) {
        for (title, name) in &rendered.picks {
            self.rotation.record(title, name); // now it really is on the device
        }
    }

    fn restore(&self, saved: &Value) {
        self.rotation.restore(saved);
    }

    fn snapshot(&self) -> Map<String, Value> {
        self.rotation.snapshot()
    }
}

#[cfg(test)]
mod tests;
