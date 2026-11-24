//! The device's backlight: brightness, its own night schedule, and dimming while the computer is locked.

use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use super::persistence::Part;
use super::screen::Screen;
use super::{config, Callback, Notify};
use crate::core::activity::hour12;
use crate::util::{in_background, LockExt};

/// The device's own night schedule (see `Display::set_night_mode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Night {
    pub enabled: bool,
    pub start: i64,
    pub end: i64,
    pub level: i64,
}

impl Default for Night {
    fn default() -> Self {
        let (enabled, start, end, level) = config::NIGHT_DEFAULT;
        Self { enabled, start, end, level }
    }
}

struct Inner {
    brightness: Option<i64>, // the day level we last set; the device can't be asked
    night: Night,
    dim_on_lock: bool, // turn the screen's backlight down while the computer is locked
}

pub struct Backlight {
    screen: Arc<Screen>,
    notify: Notify,
    save: Callback,
    refresh_menu: Callback,
    inner: Mutex<Inner>,
    command_line: (Option<(i64, i64)>, Option<i64>), // --night START-END, --night-brightness
}

impl Backlight {
    pub fn new(screen: Arc<Screen>, notify: Notify, save: Callback, refresh_menu: Callback, night_hours: Option<(i64, i64)>, night_level: Option<i64>) -> Arc<Self> {
        Arc::new(Self {
            screen,
            notify,
            save,
            refresh_menu,
            inner: Mutex::new(Inner { brightness: None, night: Night::default(), dim_on_lock: false }),
            command_line: (night_hours, night_level),
        })
    }

    pub fn brightness(&self) -> Option<i64> {
        self.inner.lk().brightness
    }
    pub fn night(&self) -> Night {
        self.inner.lk().night.clone()
    }
    pub fn dim_on_lock(&self) -> bool {
        self.inner.lk().dim_on_lock
    }

    /// Send a backlight level to the device. The device serves one request at a time, so an upload in flight is cancelled
    /// first (the worker redoes it). `quiet`: it isn't something you asked for (the dimming when the computer locks), so a
    /// screen that doesn't answer is logged but not announced.
    pub fn apply_brightness(&self, level: i64, quiet: bool) -> bool {
        if self.screen.ip().is_empty() {
            return false;
        }
        if quiet && self.screen.is_offline() {
            log::info!("not setting the brightness: the screen isn't answering");
            return false;
        }
        let Some(device) = self.screen.device() else { return false };
        self.screen.cancel_upload();
        let result = {
            let _one_request = self.screen.push_lock.lk();
            device.set_brightness(level as f64)
        };
        match result {
            Ok(()) => true,
            Err(e) => {
                log::warn!("could not set the brightness: {e}");
                if !quiet {
                    (self.notify)("Brillo", &format!("No se pudo cambiar el brillo: {e}"));
                }
                false
            }
        }
    }

    /// Menu: the normal (day) brightness. Remembered, because the device can't tell us what it's set to.
    pub fn set_brightness(self: &Arc<Self>, level: i64) {
        log::info!("brightness -> {level}");
        self.screen.touch_click(); // leave the device free for this request
        if self.apply_brightness(level, false) {
            self.inner.lk().brightness = Some(level);
            (self.save)();
            (self.refresh_menu)();
            if self.night().enabled {
                // the schedule carries the day level too, keep it in step
                let me = self.clone();
                in_background(move || {
                    me.apply_night();
                });
            }
        }
    }

    pub fn night_label(&self) -> String {
        let n = self.night();
        format!("Modo nocturno ({} - {}, brillo {} %)", hour12(n.start.rem_euclid(24) as usize), hour12(n.end.rem_euclid(24) as usize), n.level)
    }

    pub fn apply_night(&self) -> bool {
        let (night, brightness) = {
            let i = self.inner.lk();
            (i.night.clone(), i.brightness)
        };
        let (Some(device), Some(day)) = (self.screen.device(), brightness) else { return false };
        if self.screen.ip().is_empty() {
            return false;
        }
        self.screen.cancel_upload();
        let result = {
            let _one_request = self.screen.push_lock.lk();
            device.set_night_mode(night.start, night.end, night.level as f64, day as f64, night.enabled)
        };
        match result {
            Ok(()) => true,
            Err(e) => {
                log::warn!("could not set the night mode: {e}");
                (self.notify)("Modo nocturno", &format!("No se pudo cambiar el modo nocturno: {e}"));
                false
            }
        }
    }

    /// Night mode and dimming restore the normal brightness afterwards, and the device can't report it: it has to be one
    /// we set. Ask for that first instead of guessing and changing it behind the user's back.
    fn need_day_brightness(&self, what: &str) -> bool {
        if self.brightness().is_some() {
            return true;
        }
        (self.notify)(what, "Elige primero tu brillo normal en Pantalla > Brillo, para que pueda restaurarlo.");
        false
    }

    /// Menu: the device's own night schedule (works even with the computer off).
    pub fn toggle_night(&self) {
        let enable = !self.night().enabled;
        if enable && !self.need_day_brightness("Modo nocturno") {
            return;
        }
        self.screen.touch_click();
        self.inner.lk().night.enabled = enable;
        if self.apply_night() {
            log::info!("night mode {}", if enable { "on" } else { "off" });
        } else {
            self.inner.lk().night.enabled = !enable; // the device didn't take it
        }
        (self.save)();
        (self.refresh_menu)();
    }

    pub fn toggle_dim_on_lock(&self) {
        if !self.dim_on_lock() && !self.need_day_brightness("Atenuar al bloquear") {
            return;
        }
        let now = {
            let mut i = self.inner.lk();
            i.dim_on_lock = !i.dim_on_lock;
            i.dim_on_lock
        };
        log::info!("dim when locked {}", if now { "on" } else { "off" });
        (self.save)();
        (self.refresh_menu)();
    }

    /// The computer got locked or unlocked: turn the backlight down, or back to the day level.
    pub fn on_lock_change(self: &Arc<Self>, locked: bool) {
        let (dim, brightness) = {
            let i = self.inner.lk();
            (i.dim_on_lock, i.brightness)
        };
        if let (true, Some(day)) = (dim, brightness) {
            let me = self.clone();
            in_background(move || {
                me.apply_brightness(if locked { config::LOCK_DIM_LEVEL } else { day }, true);
            });
        }
    }
}

impl Part for Backlight {
    fn name(&self) -> &'static str {
        "backlight"
    }

    fn restore(&self, saved: &Value) {
        let mut i = self.inner.lk();
        i.dim_on_lock = saved.get("dim_on_lock").and_then(Value::as_bool).unwrap_or(false);
        i.brightness = saved.get("brightness").and_then(Value::as_i64);
        i.night = Night::default();
        if let Some(night) = saved.get("night").and_then(Value::as_object) {
            if let Some(v) = night.get("enabled").and_then(Value::as_bool) {
                i.night.enabled = v;
            }
            let number = |key: &str| night.get(key).and_then(Value::as_i64);
            i.night.start = number("start").unwrap_or(i.night.start);
            i.night.end = number("end").unwrap_or(i.night.end);
            i.night.level = number("level").unwrap_or(i.night.level);
        }
        // --night START-END / --night-brightness: what the menu's night mode uses
        let (hours, level) = self.command_line;
        if let Some((start, end)) = hours {
            i.night.start = start;
            i.night.end = end;
        }
        if let Some(level) = level {
            i.night.level = level;
        }
    }

    fn snapshot(&self) -> Map<String, Value> {
        let i = self.inner.lk();
        let mut m = Map::new();
        m.insert("brightness".into(), json!(i.brightness));
        m.insert("night".into(), json!({"enabled": i.night.enabled, "start": i.night.start, "end": i.night.end, "level": i.night.level}));
        m.insert("dim_on_lock".into(), json!(i.dim_on_lock));
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::screen::tests::rig;
    use crate::core::errors::DeviceError;

    fn backlight(hours: Option<(i64, i64)>, level: Option<i64>) -> (Arc<Backlight>, crate::application::screen::tests::Rig) {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({}));
        let b = Backlight::new(r.screen.clone(), r.said.notify(), Arc::new(|| {}), Arc::new(|| {}), hours, level);
        b.restore(&json!({}));
        (b, r)
    }

    #[test]
    fn the_menu_brightness_is_sent_and_remembered_only_if_the_device_took_it() {
        let (b, r) = backlight(None, None);
        b.set_brightness(50);
        assert_eq!((b.brightness(), r.display.calls().last().cloned()), (Some(50), Some("brightness 50".into())));
        r.display.failing_with(Some(DeviceError::Offline("no route".into())));
        b.set_brightness(10);
        assert_eq!(b.brightness(), Some(50), "the device refused: still 50");
        assert_eq!(r.said.titles(), ["Brillo"]);
    }

    #[test]
    fn night_mode_and_dimming_need_a_known_day_level_first() {
        let (b, r) = backlight(None, None);
        b.toggle_night();
        b.toggle_dim_on_lock();
        assert!(!b.night().enabled && !b.dim_on_lock());
        assert_eq!(r.said.titles(), ["Modo nocturno", "Atenuar al bloquear"]);
        b.set_brightness(75);
        b.toggle_night();
        assert!(b.night().enabled);
        assert_eq!(r.display.calls().last().unwrap(), "night 22 7 10 75 true");
        r.display.failing_with(Some(DeviceError::Offline("x".into())));
        b.toggle_night();
        assert!(b.night().enabled, "the device didn't take it off: the switch goes back");
    }

    #[test]
    fn dimming_on_lock_goes_down_and_back_to_the_day_level_without_nagging() {
        let (b, r) = backlight(None, None);
        b.set_brightness(75);
        b.toggle_dim_on_lock();
        r.display.calls.lock().unwrap().clear();
        b.on_lock_change(true);
        b.on_lock_change(false);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let calls = r.display.calls();
        assert!(calls.contains(&"brightness 0".to_string()) && calls.contains(&"brightness 75".to_string()), "{calls:?}");
        r.screen.mark_offline("x");
        assert!(!b.apply_brightness(0, true), "quiet: not even tried while the screen is gone");
        assert!(r.said.titles().is_empty(), "and nothing announced");
    }

    #[test]
    fn it_is_remembered_and_the_command_line_wins_for_the_night() {
        let (b, _) = backlight(Some((23, 6)), Some(5));
        b.restore(&json!({"brightness": 40, "dim_on_lock": true, "night": {"enabled": true, "start": 20, "end": 8, "level": 30}}));
        assert_eq!((b.brightness(), b.dim_on_lock()), (Some(40), true));
        assert_eq!(b.night(), Night { enabled: true, start: 23, end: 6, level: 5 });
        assert_eq!(b.night_label(), "Modo nocturno (11 PM - 6 AM, brillo 5 %)");
        assert_eq!(b.snapshot()["night"], json!({"enabled": true, "start": 23, "end": 6, "level": 5}));
        let (plain, _) = backlight(None, None);
        plain.restore(&json!({"brightness": true, "night": "junk"}));
        assert_eq!((plain.brightness(), plain.night()), (None, Night::default()), "a bool is not a level");
    }
}
