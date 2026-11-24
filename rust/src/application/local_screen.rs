//! The pictures of the screen, kept on the computer too: what the simulated screen shows when there is no GeekMagic to show
//! them (it is switched off, on another network, not found yet...).
//!
//! Every picture drawn for the device is remembered here, and while the device can't be reached the pictures are still drawn
//! (see `Screen::show_locally`), so the tray icon can be the screen: a click opens a window that plays the active view's GIF.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use crate::core::model::Data;
use crate::util::LockExt;

/// A fingerprint of what a picture is drawn from: equal data, equal fingerprint.
pub fn fingerprint(data: &Data) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{data:?}").hash(&mut hasher);
    hasher.finish()
}

/// (view, version, GIF) of the view that should be on screen.
pub type Latest = (String, u64, Arc<Vec<u8>>);

#[derive(Default)]
struct Inner {
    images: HashMap<String, (u64, Arc<Vec<u8>>)>, // view -> (version, its GIF)
    drawn_from: HashMap<String, u64>,             // view -> fingerprint of what that picture was drawn from
    version: u64,
}

pub struct LocalScreen {
    active_key: Arc<dyn Fn() -> String + Send + Sync>,
    inner: Mutex<Inner>,
}

impl LocalScreen {
    pub fn new(active_key: Arc<dyn Fn() -> String + Send + Sync>) -> Self {
        Self { active_key, inner: Mutex::new(Inner::default()) }
    }

    /// `gif` is now the picture of the view `key`, drawn from what `drawn_from` fingerprints.
    pub fn remember(&self, key: &str, gif: Vec<u8>, drawn_from: u64) {
        let mut i = self.inner.lk();
        i.version += 1;
        let version = i.version;
        i.images.insert(key.to_string(), (version, Arc::new(gif)));
        i.drawn_from.insert(key.to_string(), drawn_from);
    }

    /// Is the picture of `key` already drawn from exactly this? (Then there is no point in drawing it again.)
    pub fn is_current(&self, key: &str, drawn_from: u64) -> bool {
        let i = self.inner.lk();
        i.images.contains_key(key) && i.drawn_from.get(key) == Some(&drawn_from)
    }

    /// The picture of the view that should be on screen, None if it has none yet. Its version changes whenever the picture
    /// does, so a window can tell without comparing bytes.
    pub fn latest(&self) -> Option<Latest> {
        let key = (self.active_key)();
        let (version, gif) = self.inner.lk().images.get(&key).cloned()?;
        Some((key, version, gif))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Usage, Window};

    #[test]
    fn it_keeps_the_picture_of_the_active_view_and_its_version_changes_with_it() {
        let active = Arc::new(Mutex::new("claude".to_string()));
        let a = active.clone();
        let local = LocalScreen::new(Arc::new(move || a.lk().clone()));
        assert!(local.latest().is_none(), "nothing drawn yet");
        local.remember("claude", b"one".to_vec(), 1);
        local.remember("codex", b"other".to_vec(), 2);
        let (view, first, gif) = local.latest().unwrap();
        assert_eq!((view.as_str(), gif.as_slice()), ("claude", b"one".as_slice()));
        local.remember("claude", b"two".to_vec(), 3);
        assert!(local.latest().unwrap().1 > first);
        *active.lk() = "codex".into();
        assert_eq!(local.latest().unwrap().2.as_slice(), b"other");
        *active.lk() = "split".into();
        assert!(local.latest().is_none(), "a view that was never drawn");
    }

    #[test]
    fn it_knows_when_a_picture_is_already_drawn_from_this_data() {
        let local = LocalScreen::new(Arc::new(|| "claude".into()));
        assert!(!local.is_current("claude", 7));
        local.remember("claude", b"gif".to_vec(), 7);
        assert!(local.is_current("claude", 7) && !local.is_current("claude", 8) && !local.is_current("codex", 7));
    }

    #[test]
    fn the_fingerprint_follows_the_data() {
        let now = chrono::Local::now();
        let reading = |pct| Data::One(Usage::new("Claude", Window { pct: Some(pct), ..Default::default() }, Window::default(), now));
        assert_eq!(fingerprint(&reading(10.0)), fingerprint(&reading(10.0)));
        assert_ne!(fingerprint(&reading(10.0)), fingerprint(&reading(11.0)));
        let Data::One(mut working) = reading(10.0) else { unreachable!() };
        working.working = true;
        assert_ne!(fingerprint(&reading(10.0)), fingerprint(&Data::One(working)), "the agent's flag changes the picture");
    }
}
