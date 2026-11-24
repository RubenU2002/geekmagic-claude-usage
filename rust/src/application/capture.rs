//! Taking a picture of what the screen is showing, and saving it where you say.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Local;

use super::screen::Screen;
use super::{image_files, Notify};
use crate::core::ports::Renderer;
use crate::util::{in_background, LockExt};

pub struct ScreenCapture {
    screen: Arc<Screen>,
    renderer: Arc<dyn Renderer>,
    ask_save_path: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
    notify: Notify,
}

impl ScreenCapture {
    pub fn new(screen: Arc<Screen>, renderer: Arc<dyn Renderer>, ask_save_path: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>, notify: Notify) -> Arc<Self> {
        Arc::new(Self { screen, renderer, ask_save_path, notify })
    }

    /// The menu's command: it asks where to save, which takes as long as you take, so it runs off the caller's thread.
    pub fn take(self: &Arc<Self>) {
        let me = self.clone();
        in_background(move || {
            me.capture();
        });
    }

    /// Save the image on the screen (it is read back from the device, so it is what is really there) as a PNG, or as the
    /// animated GIF itself when the name you choose ends in .gif. Returns the file, None if nothing was saved.
    pub fn capture(&self) -> Option<PathBuf> {
        let s = &self.screen;
        let key = s.active_key();
        let slot = s.slot_of(&key);
        if s.ip().is_empty() || s.is_offline() {
            return self.failed("la pantalla no responde ahora");
        }
        let Some(slot) = slot else { return self.failed("todavía no hay una imagen en la pantalla") };
        let stamp = Local::now().format("%Y%m%d-%H%M%S");
        let chosen = (self.ask_save_path)(&format!("geekmagic-{key}-{stamp}.png"))?;
        if chosen.is_empty() {
            return None;
        }
        let path = Path::new(&chosen);
        let saved = (|| -> Result<(), String> {
            let device = s.device().ok_or("la pantalla no responde ahora")?;
            let gif = {
                let _one_request = s.push_lock.lk(); // the device serves one request at a time
                device.download_image(&image_files::slot_file(&key, &slot)).map_err(|e| e.to_string())?
            };
            let is_gif = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif"));
            let bytes = if is_gif { gif } else { self.renderer.to_png(&gif)? };
            std::fs::write(path, bytes).map_err(|e| e.to_string())
        })();
        if let Err(e) = saved {
            log::warn!("could not save the screenshot: {e}");
            return self.failed(&e);
        }
        log::info!("screenshot saved: {}", path.display());
        (self.notify)("Pantallazo guardado", &path.display().to_string());
        Some(path.to_path_buf())
    }

    fn failed(&self, why: &str) -> Option<PathBuf> {
        (self.notify)("Pantallazo", &format!("No se pudo guardar: {why}"));
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::persistence::Part;
    use crate::application::screen::tests::rig;
    use serde_json::json;

    fn capture_rig(answer: Option<String>) -> (Arc<ScreenCapture>, crate::application::screen::tests::Rig) {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({"slots": {"claude": "b"}}));
        *r.display.downloads.lk() = b"GIF-BYTES".to_vec();
        let capture = ScreenCapture::new(r.screen.clone(), r.renderer.clone(), Arc::new(move |_| answer.clone()), r.said.notify());
        (capture, r)
    }

    fn temp(name: &str) -> String {
        std::env::temp_dir().join(format!("gm-capture-{}-{name}", std::process::id())).display().to_string()
    }

    #[test]
    fn a_png_by_default_and_the_animated_gif_when_named_so() {
        let png = temp("a.png");
        let (c, r) = capture_rig(Some(png.clone()));
        assert!(c.capture().is_some());
        assert_eq!(std::fs::read(&png).unwrap(), b"PNG:GIF-BYTES", "read back from the device, then made a still");
        assert!(r.display.calls().contains(&"download claude-usage-b.gif".to_string()));
        assert_eq!(r.said.titles(), ["Pantallazo guardado"]);
        let gif = temp("b.GIF");
        let (c, _) = capture_rig(Some(gif.clone()));
        assert!(c.capture().is_some());
        assert_eq!(std::fs::read(&gif).unwrap(), b"GIF-BYTES", "the file as it is on the device");
        let _ = (std::fs::remove_file(png), std::fs::remove_file(gif));
    }

    #[test]
    fn nothing_is_saved_when_cancelled_offline_or_not_there_yet() {
        let (c, r) = capture_rig(None);
        assert!(c.capture().is_none() && r.said.all().is_empty(), "cancelling the dialog is not a failure");
        r.screen.mark_offline("x");
        assert!(c.capture().is_none());
        assert_eq!(r.said.all()[0], ("Pantallazo".to_string(), "No se pudo guardar: la pantalla no responde ahora".to_string()));
        let (c, r) = capture_rig(Some(temp("never.png")));
        *r.active.lk() = "codex".into(); // nothing stored for the view on screen
        assert!(c.capture().is_none());
        assert!(r.said.all()[0].1.contains("todavía no hay una imagen"));
        let (c, r) = capture_rig(Some(temp("fails.png")));
        r.display.failing_with(Some(crate::core::errors::DeviceError::Offline("cut off".into())));
        assert!(c.capture().is_none());
        assert!(r.said.all()[0].1.contains("cut off"));
    }
}
