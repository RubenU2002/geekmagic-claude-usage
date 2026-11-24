//! The screen itself: where it is on the network, which of its stored images is current, and putting pictures on it.
//!
//! Every view has two image files on the device (slot "a" and "b") and uploads alternate between them, so an upload cut
//! short by a click can only damage the spare one: the one on screen, and the one a switch will show, stay intact.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{json, Map, Value};

use super::local_screen::{fingerprint, LocalScreen};
use super::persistence::Part;
use super::{config, image_files, Callback, Notify};
use crate::core::errors::DeviceError;
use crate::core::model::Data;
use crate::core::ports::{DeviceLocator, Display, Renderer};
use crate::core::registry::ProviderRegistry;
use crate::core::views::is_panel_key;
use crate::util::{mono, truncate_chars, Event, LockExt};

/// How a delivery to the screen ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    /// A click got in the way.
    Cancelled,
    Offline,
    Error,
}

/// What the screen asks of whoever builds it.
pub struct ScreenHooks {
    /// (ip) -> the screen at that address.
    pub display_factory: Arc<dyn Fn(&str) -> Arc<dyn Display> + Send + Sync>,
    pub active_key: Arc<dyn Fn() -> String + Send + Sync>,
    /// (key, data) -> data with the "its agent is working" flags set.
    pub flag: Arc<dyn Fn(&str, Data) -> Data + Send + Sync>,
    pub notify: Notify,
    pub save: Callback,
    pub set_title: Arc<dyn Fn(&str) + Send + Sync>,
    pub tooltip: Arc<dyn Fn() -> String + Send + Sync>,
    /// The tray menu shows where the screen is.
    pub refresh_menu: Callback,
}

#[derive(Default)]
struct State {
    saved_ip: Option<String>, // last address the device answered on, see restore
    ip: String,               // "" until the device is found
    device: Option<Arc<dyn Display>>,
    slots: HashMap<String, String>, // view -> "a"/"b": the file holding its complete image on the device
    outdated: HashSet<String>,      // views whose stored image was drawn by an older version of the code
    last_upload: HashMap<String, f64>,
    last_click: f64,
    last_scan: f64,
    offline: bool, // the device didn't answer on the last attempt
    offline_since: Option<f64>,
    offline_noticed: bool, // this outage has been announced (so its end is too)
    shown_failed: bool,    // the last attempt to put a remembered image on screen didn't get through
    error_shown: HashMap<String, String>, // provider -> the error its image on the device says
}

pub struct Screen {
    animation: String, // "auto" / "random", or the name of one animation to pin
    providers: Arc<ProviderRegistry>,
    renderer: Arc<dyn Renderer>,
    locator: Arc<dyn DeviceLocator>,
    hooks: ScreenHooks,
    wake: Arc<Event>,
    given_ip: Option<String>, // --ip
    render_id: String,        // identifies the code that draws the screens, see snapshot
    state: Mutex<State>,
    cancel: Mutex<Arc<AtomicBool>>, // set by a click: abort the upload in flight
    /// The device serves one request at a time.
    pub push_lock: Mutex<()>,
    /// The same pictures, for the simulated screen when the device isn't there.
    pub local: Arc<LocalScreen>,
}

impl Screen {
    pub fn new(
        ip: Option<String>,
        animation: &str,
        providers: Arc<ProviderRegistry>,
        renderer: Arc<dyn Renderer>,
        locator: Arc<dyn DeviceLocator>,
        hooks: ScreenHooks,
        wake: Arc<Event>,
    ) -> Self {
        let render_id = renderer.version().to_string();
        let local = Arc::new(LocalScreen::new(hooks.active_key.clone()));
        Self {
            animation: animation.to_string(),
            providers,
            renderer,
            locator,
            hooks,
            wake,
            given_ip: ip.filter(|i| !i.is_empty()),
            render_id,
            state: Mutex::new(State::default()),
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
            push_lock: Mutex::new(()),
            local,
        }
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.state.lk()
    }

    // --- what others ask ------------------------------------------------------------------------------------

    pub fn ip(&self) -> String {
        self.st().ip.clone()
    }
    /// What the screen should be showing: a provider, or a view of two.
    pub fn active_key(&self) -> String {
        (self.hooks.active_key)()
    }
    pub fn is_offline(&self) -> bool {
        self.st().offline
    }
    pub fn device(&self) -> Option<Arc<dyn Display>> {
        self.st().device.clone()
    }
    pub fn slot_of(&self, key: &str) -> Option<String> {
        self.st().slots.get(key).cloned()
    }
    pub fn has_slot(&self, key: &str) -> bool {
        self.st().slots.contains_key(key)
    }
    pub fn is_outdated(&self, key: &str) -> bool {
        self.st().outdated.contains(key)
    }
    /// The views whose stored image is out of date, in a steady order.
    pub fn outdated_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.st().outdated.iter().cloned().collect();
        keys.sort();
        keys
    }
    pub fn error_shown(&self, provider: &str) -> Option<String> {
        self.st().error_shown.get(provider).cloned()
    }
    pub fn clear_error_shown(&self, provider: &str) {
        self.st().error_shown.remove(provider);
    }
    pub fn last_upload(&self, key: &str) -> f64 {
        self.st().last_upload.get(key).copied().unwrap_or(0.0)
    }
    pub fn shown_failed(&self) -> bool {
        self.st().shown_failed
    }
    /// Leave the device free for the request that is about to be made.
    pub fn touch_click(&self) {
        self.st().last_click = mono();
    }
    pub fn last_click(&self) -> f64 {
        self.st().last_click
    }
    /// Abort the upload in flight (the device serves one request at a time).
    pub fn cancel_upload(&self) {
        self.cancel.lk().store(true, Ordering::SeqCst);
    }

    // --- remembered -----------------------------------------------------------------------------------------

    fn set_ip_quietly(&self, ip: &str) {
        let device = (!ip.is_empty()).then(|| (self.hooks.display_factory)(ip));
        let mut st = self.st();
        st.ip = ip.to_string();
        st.device = device;
    }

    // --- finding the device ---------------------------------------------------------------------------------

    pub fn set_ip(&self, ip: &str, announce: bool) {
        let changed = ip != self.ip();
        self.set_ip_quietly(ip);
        (self.hooks.save)();
        if announce && changed {
            log::info!("device found at {ip}");
            (self.hooks.notify)("Pantalla encontrada", &format!("GeekMagic en {ip}"));
            self.st().offline_noticed = false; // (that said it: no second "connected" notice)
        }
        if changed {
            (self.hooks.refresh_menu)();
            self.sync_with_device(); // a different device may not have our images
        }
    }

    /// At startup: use --ip or the remembered address if it answers, otherwise look for the device.
    pub fn resolve_device(&self) {
        let (ip, saved) = {
            let st = self.st();
            (st.ip.clone(), st.saved_ip.clone())
        };
        let mut candidates: Vec<String> = Vec::new();
        for c in [ip.clone(), saved.unwrap_or_default()] {
            if !c.is_empty() && !candidates.contains(&c) {
                candidates.push(c);
            }
        }
        for candidate in candidates {
            if self.locator.probe(&candidate, 2.0) {
                self.set_ip_quietly(&candidate);
                return;
            }
        }
        (self.hooks.set_title)("GeekMagic: buscando la pantalla en la red...");
        self.st().last_scan = mono();
        match self.locator.find_device((!ip.is_empty()).then_some(ip.as_str())) {
            Some(found) => self.set_ip(&found, true),
            None => log::warn!(
                "no GeekMagic device found yet (tried {}); will keep looking",
                if ip.is_empty() { "a network scan" } else { &ip }
            ),
        }
    }

    /// Called while the device doesn't answer: it may have a new IP, so look for it now and then.
    pub fn maybe_rediscover(&self) {
        let now = mono();
        let (ip, due) = {
            let st = self.st();
            let due = if st.ip.is_empty() {
                now - st.last_scan >= config::DISCOVER_RETRY
            } else {
                st.offline_since.is_some_and(|s| now - s >= config::REDISCOVER_AFTER) && now - st.last_scan >= config::REDISCOVER_EVERY
            };
            (st.ip.clone(), due)
        };
        if !due {
            return;
        }
        self.st().last_scan = now;
        match self.locator.find_device((!ip.is_empty()).then_some(ip.as_str())) {
            Some(found) if found != ip => self.set_ip(&found, true),
            Some(_) => {}
            None => self.announce_outage(),
        }
    }

    /// Once per outage, after a search for the screen found nothing: say that it's gone and the likely why (a notice on
    /// the first miss would only nag about blips).
    pub fn announce_outage(&self) {
        {
            let mut st = self.st();
            let Some(since) = st.offline_since else { return };
            if st.offline_noticed || !st.offline || mono() - since < config::OFFLINE_NOTICE_AFTER || st.last_scan < since {
                return;
            }
            st.offline_noticed = true;
        }
        let message = self.outage_reason();
        log::warn!("announcing: {message}");
        (self.hooks.notify)("Sin conexión con la pantalla", &message);
    }

    pub fn outage_reason(&self) -> String {
        let networks = self.locator.networks();
        let ip = self.ip();
        let Some(main) = networks.first() else { return "Este PC no tiene conexión de red.".into() };
        if !ip.is_empty() && !networks.iter().any(|p| ip.starts_with(p.as_str())) {
            return format!("El PC está en la red {main}x y la pantalla estaba en {ip}, que es otra red: ¿cambió de WiFi o de router?");
        }
        let at = if ip.is_empty() { String::new() } else { format!(" en {ip}") };
        format!("No responde{at} ni aparece en la red {main}x: ¿está apagada, sin WiFi o con otra IP?")
    }

    /// Where the screen is, for the tray menu.
    pub fn status_line(&self) -> String {
        let st = self.st();
        if st.ip.is_empty() {
            "Pantalla: buscando...".into()
        } else if st.offline {
            format!("Pantalla: sin conexión ({})", st.ip)
        } else {
            format!("Pantalla: {}", st.ip)
        }
    }

    /// The same, for the icon's tooltip.
    pub fn status_note(&self) -> String {
        let st = self.st();
        if st.ip.is_empty() {
            "buscando la pantalla".into()
        } else if st.offline {
            "pantalla sin conexión".into()
        } else {
            format!("pantalla {}", st.ip)
        }
    }

    // --- what's on the device -------------------------------------------------------------------------------

    /// Upload fresh data into the view's spare slot, then pin it if it's still the active view. `key` is a provider
    /// (`data` is its Usage, or the ErrorScreen saying why it can't be read) or one of the views of two (`data` is the
    /// list of the two Usage). `Ok(false)`: not uploaded (a click got in the way).
    pub fn upload(&self, key: &str, data: Data) -> Result<bool, DeviceError> {
        if mono() - self.last_click() < config::SETTLE_SECONDS {
            // A click just happened. Cancelling only stops the upload in flight, and the rest of this cycle's uploads
            // would start straight away and keep the device (which serves one request at a time) busy for seconds
            // while the click's own request waits. Stand down; the worker retries once clicking settles.
            self.wake.set();
            return Ok(false);
        }
        let data = (self.hooks.flag)(key, data);
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.lk() = cancel.clone();
        let _one_request = self.push_lock.lk();
        let (spare, device) = {
            let st = self.st();
            (if st.slots.get(key).is_some_and(|s| s == "a") { "b" } else { "a" }, st.device.clone())
        };
        let device = device.ok_or_else(|| DeviceError::Offline("no device address yet".into()))?;
        let filename = image_files::slot_file(key, spare);
        log::info!("uploading {key} -> {filename}");
        let rendered = self.renderer.render(key, &data, &self.animation).map_err(DeviceError::Other)?;
        // (the simulated screen shows it whether or not it gets there)
        self.local.remember(key, rendered.gif.clone(), fingerprint(&data));
        match device.upload(&rendered.gif, &filename, "image/gif", Some(&cancel)) {
            Err(DeviceError::Cancelled) => {
                log::info!("upload cancelled ({key})");
                self.wake.set(); // retry once the clicking settles
                return Ok(false);
            }
            other => other?,
        }
        self.renderer.confirm(&rendered); // now it really is on the device
        {
            let mut st = self.st();
            st.slots.insert(key.to_string(), spare.to_string());
            st.outdated.remove(key);
            if let (true, Data::Error(e)) = (self.providers.contains(key), &data) {
                st.error_shown.insert(key.to_string(), e.message.clone());
            }
        }
        (self.hooks.save)();
        self.st().last_upload.insert(key.to_string(), mono());
        if key == (self.hooks.active_key)() {
            // checked after the slow upload, in case the user switched meanwhile
            device.show_image(&filename)?;
        }
        Ok(true)
    }

    /// Forget remembered images the device no longer has, and drop leftovers of the old single-file scheme.
    pub fn sync_with_device(&self) {
        let Some(device) = self.device() else { return };
        let Ok(stored) = device.list_images() else { return }; // unreachable right now: keep what we remember
        {
            let mut st = self.st();
            let gone: Vec<String> =
                st.slots.iter().filter(|(k, s)| !stored.contains(&image_files::slot_file(k, s))).map(|(k, _)| k.clone()).collect();
            for key in gone {
                log::info!("{key} image is gone from the device, will re-upload");
                st.slots.remove(&key);
            }
        }
        for key in self.providers.keys() {
            let legacy = image_files::image_name(&key);
            if stored.contains(&legacy) && device.delete_image(&legacy).is_err() {
                return;
            }
        }
        (self.hooks.save)();
    }

    /// Instant switch: the view's last image is already on the device, so just pin it.
    pub fn show_uploaded(&self, key: &str) {
        let Some(slot) = self.slot_of(key) else {
            log::info!("no {key} image on the device yet; it appears after its first upload");
            return;
        };
        let _one_request = self.push_lock.lk();
        if key != (self.hooks.active_key)() {
            return;
        }
        let shown = match self.device() {
            Some(device) => device.show_image(&image_files::slot_file(key, &slot)),
            None => Err(DeviceError::Offline("no device address yet".into())),
        };
        match shown {
            Ok(()) => {
                self.st().shown_failed = false;
                log::info!("showed {key}");
            }
            // the device not answering is routine (it's off, or being searched for)
            Err(e) => {
                self.st().shown_failed = true;
                log::warn!("could not show {key}: {e}");
            }
        }
    }

    // --- the device being there or not ----------------------------------------------------------------------

    pub fn mark_offline(&self, reason: &str) {
        let first = {
            let mut st = self.st();
            let first = !st.offline;
            if first {
                st.offline = true;
                st.offline_since = Some(mono());
            }
            first
        };
        if first {
            log::warn!("device unreachable ({reason}), retrying every {}s", config::RETRY_SECONDS);
            (self.hooks.refresh_menu)();
        }
        (self.hooks.set_title)("GeekMagic: sin conexión con la pantalla, reintentando");
    }

    /// While offline: pin the remembered view with one small request. If it works the device is back and its screen is
    /// on the right view again at once, without waiting for a whole upload; if not, we know it without trying a (slow)
    /// upload.
    pub fn device_is_back(&self) -> bool {
        let key = (self.hooks.active_key)();
        let (slot, ip, device) = {
            let st = self.st();
            (st.slots.get(&key).cloned(), st.ip.clone(), st.device.clone())
        };
        let outcome = {
            let _one_request = self.push_lock.lk();
            match (slot, device) {
                (Some(slot), Some(device)) => device.show_image(&image_files::slot_file(&key, &slot)),
                _ if !self.locator.probe(&ip, 2.0) => Err(DeviceError::Offline("no answer".into())),
                _ => Ok(()),
            }
        };
        match outcome {
            Ok(()) => {
                self.came_back();
                true
            }
            Err(e) => {
                self.mark_offline(&e.to_string());
                false
            }
        }
    }

    pub fn came_back(&self) {
        log::info!("device is back");
        let (noticed, ip) = {
            let mut st = self.st();
            st.offline = false;
            st.offline_since = None;
            (std::mem::take(&mut st.offline_noticed), st.ip.clone())
        };
        if noticed {
            (self.hooks.notify)("Pantalla conectada", &format!("GeekMagic responde de nuevo en {ip}"));
        }
        (self.hooks.refresh_menu)();
        (self.hooks.set_title)(&(self.hooks.tooltip)());
    }

    /// Draw the view for the simulated screen only (the device can't be reached), unless the picture it already has was drawn
    /// from this very data: a retry every few seconds must not redraw the same thing.
    pub fn show_locally(&self, key: &str, data: Data) {
        let data = (self.hooks.flag)(key, data);
        let drawn_from = fingerprint(&data);
        if self.local.is_current(key, drawn_from) {
            return;
        }
        // (the simulated screen is a convenience: a failure to draw is logged, never raised)
        match self.renderer.render(key, &data, &self.animation) {
            Ok(rendered) => self.local.remember(key, rendered.gif, drawn_from),
            Err(e) => log::warn!("could not draw {key} for the simulated screen: {e}"),
        }
    }

    /// Upload `data` for a view. While the device can't be reached the picture is still drawn (see `show_locally`), for the
    /// simulated screen.
    pub fn deliver(&self, key: &str, data: Data) -> Outcome {
        if self.ip().is_empty() {
            self.mark_offline("no device address yet");
            self.show_locally(key, data);
            return Outcome::Offline;
        }
        if self.is_offline() && !self.device_is_back() {
            self.show_locally(key, data);
            return Outcome::Offline;
        }
        match self.upload(key, data) {
            Ok(true) => {}
            Ok(false) | Err(DeviceError::Cancelled) => return Outcome::Cancelled,
            Err(DeviceError::Offline(e)) => {
                // the device is off or unreachable
                self.mark_offline(&e);
                return Outcome::Offline;
            }
            Err(DeviceError::Other(e)) => {
                log::error!("upload failed ({key}): {e}");
                (self.hooks.set_title)(&truncate_chars(&format!("GeekMagic: {e}"), 120));
                return Outcome::Error;
            }
        }
        log::info!("pushed {key}");
        if self.is_offline() {
            self.came_back();
        }
        (self.hooks.set_title)(&(self.hooks.tooltip)());
        Outcome::Ok
    }

    // --- clicks ---------------------------------------------------------------------------------------------

    /// A click: abort any upload in flight (the device serves one request at a time) and keep new ones off until
    /// clicking settles.
    pub fn note_click(&self) {
        self.touch_click();
        self.cancel_upload();
    }
}

impl Part for Screen {
    fn name(&self) -> &'static str {
        "screen"
    }

    fn restore(&self, saved: &Value) {
        let saved_ip = saved.get("ip").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        // older files were just the slot map
        let slot_source = saved.get("slots").unwrap_or(saved).as_object().cloned().unwrap_or_default();
        let wanted = |key: &str| self.providers.contains(key) || is_panel_key(key);
        let slots: HashMap<String, String> = slot_source
            .iter()
            .filter_map(|(k, v)| v.as_str().filter(|s| *s == "a" || *s == "b").map(|s| (k.clone(), s.to_string())))
            .filter(|(k, _)| wanted(k))
            .collect();
        let current = saved.get("render").and_then(Value::as_str) == Some(self.render_id.as_str());
        let ip = self.given_ip.clone().or(saved_ip.clone()).unwrap_or_default();
        {
            let mut st = self.st();
            st.saved_ip = saved_ip;
            // still shown at once when switched to, and refreshed in the background
            st.outdated = if current { HashSet::new() } else { slots.keys().cloned().collect() };
            st.slots = slots;
        }
        self.set_ip_quietly(&ip);
    }

    fn snapshot(&self) -> Map<String, Value> {
        let st = self.st();
        let mut m = Map::new();
        m.insert("slots".into(), json!(st.slots));
        m.insert("ip".into(), if st.ip.is_empty() { Value::Null } else { json!(st.ip) });
        // only claimed once every stored image is current
        m.insert("render".into(), if st.outdated.is_empty() { json!(self.render_id) } else { Value::Null });
        m
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::application::testing::*;
    use crate::core::model::{ErrorScreen, Usage, Window};
    use crate::core::ports::Provider;
    use crate::application::usage::tests::{registry, Scripted};

    pub struct Rig {
        pub screen: Arc<Screen>,
        pub display: Arc<FakeDisplay>,
        pub locator: Arc<FakeLocator>,
        pub renderer: Arc<FakeRenderer>,
        pub said: Arc<Said>,
        pub active: Arc<Mutex<String>>,
        pub saves: Arc<Mutex<usize>>,
        pub titles: Arc<Mutex<Vec<String>>>,
        pub wake: Arc<Event>,
    }

    pub fn rig(ip: Option<&str>) -> Rig {
        let providers: Vec<Arc<dyn Provider>> = vec![Scripted::new("claude", "Claude", vec![Ok((1.0, 1.0))]), Scripted::new("codex", "Codex", vec![Ok((1.0, 1.0))])];
        let (display, locator, renderer, said) = (FakeDisplay::new("10.0.0.5"), FakeLocator::answering(&["10.0.0.5"]), Arc::new(FakeRenderer::default()), Said::new());
        let active = Arc::new(Mutex::new("claude".to_string()));
        let (saves, titles, wake) = (Arc::new(Mutex::new(0)), Arc::new(Mutex::new(Vec::new())), Arc::new(Event::new()));
        let (a, s, t, d) = (active.clone(), saves.clone(), titles.clone(), display.clone());
        let hooks = ScreenHooks {
            display_factory: Arc::new(move |_| d.clone() as Arc<dyn Display>),
            active_key: Arc::new(move || a.lk().clone()),
            flag: Arc::new(|_, data| data),
            notify: said.notify(),
            save: Arc::new(move || *s.lk() += 1),
            set_title: Arc::new(move |title| t.lk().push(title.to_string())),
            tooltip: Arc::new(|| "tooltip".into()),
            refresh_menu: Arc::new(|| {}),
        };
        let screen = Arc::new(Screen::new(ip.map(String::from), "auto", registry(providers), renderer.clone(), locator.clone(), hooks, wake.clone()));
        Rig { screen, display, locator, renderer, said, active, saves, titles, wake }
    }

    pub fn usage(title: &str) -> Data {
        Data::One(Usage::new(title, Window::default(), Window::default(), chrono::Local::now()))
    }

    #[test]
    fn uploads_alternate_between_two_slots_and_pin_the_active_view() {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({}));
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Ok);
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Ok);
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Ok);
        let calls = r.display.calls();
        assert_eq!(
            calls,
            ["upload claude-usage-a.gif", "show claude-usage-a.gif", "upload claude-usage-b.gif", "show claude-usage-b.gif", "upload claude-usage-a.gif", "show claude-usage-a.gif"]
        );
        assert_eq!(*r.renderer.confirmed.lk(), 3, "remembered once it is on the device");
        // a view that is not on screen is stored but not pinned
        assert_eq!(r.screen.deliver("codex", usage("Codex")), Outcome::Ok);
        assert_eq!(r.display.calls().last().unwrap(), "upload codex-usage-a.gif");
        assert_eq!(r.screen.slot_of("codex").as_deref(), Some("a"));
    }

    #[test]
    fn a_click_cancels_and_a_cancelled_upload_changes_nothing() {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({}));
        r.display.cancel_uploads.store(true, Ordering::SeqCst);
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Cancelled);
        assert!(r.screen.slot_of("claude").is_none(), "nothing stored");
        assert_eq!(*r.renderer.confirmed.lk(), 0, "and nothing remembered as picked");
        assert!(r.wake.is_set(), "the worker retries once clicking settles");
        // right after a click, new uploads stand down altogether
        r.display.cancel_uploads.store(false, Ordering::SeqCst);
        r.display.calls.lk().clear();
        r.screen.note_click();
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Cancelled);
        assert!(r.display.calls().is_empty());
    }

    #[test]
    fn an_unreachable_device_is_marked_offline_and_found_again_with_one_small_request() {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({"slots": {"claude": "a"}}));
        r.display.failing_with(Some(DeviceError::Offline("no route".into())));
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Offline);
        assert!(r.screen.is_offline());
        assert_eq!(r.screen.status_line(), "Pantalla: sin conexión (10.0.0.5)");
        assert!(r.titles.lk().last().unwrap().contains("sin conexión"));
        r.display.calls.lk().clear();
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Offline);
        assert_eq!(r.display.calls(), ["show claude-usage-a.gif"], "offline: only the small request, no upload");
        r.display.failing_with(None);
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Ok);
        assert!(!r.screen.is_offline());
        assert_eq!(r.titles.lk().last().unwrap(), "tooltip");
    }

    #[test]
    fn a_drawing_failure_is_an_error_not_an_outage() {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({}));
        r.renderer.fail.store(true, Ordering::SeqCst);
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Error);
        assert!(!r.screen.is_offline());
        assert!(r.titles.lk().last().unwrap().starts_with("GeekMagic: cannot draw"));
    }

    #[test]
    fn without_an_address_it_is_offline_and_a_scan_finds_it() {
        let r = rig(None);
        r.screen.restore(&json!({}));
        assert_eq!(r.screen.status_line(), "Pantalla: buscando...");
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Offline);
        *r.locator.found.lk() = Some("10.0.0.5".into());
        r.screen.resolve_device();
        assert_eq!(r.screen.ip(), "10.0.0.5");
        assert!(r.said.titles().contains(&"Pantalla encontrada".to_string()));
        assert!(*r.saves.lk() >= 1, "the address is remembered");
    }

    #[test]
    fn a_remembered_address_that_answers_is_used_quietly() {
        let r = rig(None);
        r.screen.restore(&json!({"ip": "10.0.0.5"}));
        r.screen.resolve_device();
        assert_eq!((r.screen.ip().as_str(), *r.locator.scans.lk()), ("10.0.0.5", 0));
        assert!(r.said.all().is_empty());
    }

    #[test]
    fn the_outage_is_announced_once_with_its_likely_reason() {
        let r = rig(Some("192.168.1.18"));
        r.screen.restore(&json!({}));
        r.screen.mark_offline("timeout");
        *r.locator.nets.lk() = vec!["10.0.0.".into()];
        {
            let mut st = r.screen.st();
            st.offline_since = Some(mono() - config::OFFLINE_NOTICE_AFTER - 1.0);
            st.last_scan = mono();
        }
        r.screen.announce_outage();
        r.screen.announce_outage();
        let said = r.said.all();
        assert_eq!(said.len(), 1, "once");
        assert_eq!(said[0].0, "Sin conexión con la pantalla");
        assert!(said[0].1.contains("otra red") && said[0].1.contains("10.0.0.x"), "{}", said[0].1);
        r.screen.came_back();
        assert_eq!(r.said.titles().last().unwrap(), "Pantalla conectada", "its end is announced too");
        *r.locator.nets.lk() = vec![];
        assert_eq!(r.screen.outage_reason(), "Este PC no tiene conexión de red.");
        *r.locator.nets.lk() = vec!["192.168.1.".into()];
        assert!(r.screen.outage_reason().starts_with("No responde en 192.168.1.18 ni aparece en la red 192.168.1.x"));
    }

    #[test]
    fn remembered_images_the_device_lost_are_forgotten_and_the_legacy_ones_deleted() {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({"slots": {"claude": "a", "codex": "b", "split": "a", "nonsense": "a", "stats": "z"}}));
        assert!(!r.screen.has_slot("nonsense") && !r.screen.has_slot("stats"), "unknown views and slots are dropped");
        r.display.stored.lk().extend(["claude-usage-a.gif".to_string(), "claude-usage.gif".to_string()]);
        r.screen.sync_with_device();
        assert!(r.screen.has_slot("claude") && !r.screen.has_slot("codex") && !r.screen.has_slot("split"));
        assert!(r.display.calls().contains(&"delete claude-usage.gif".to_string()));
    }

    #[test]
    fn show_uploaded_pins_only_the_view_still_on_screen_and_remembers_a_failure() {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({"slots": {"claude": "b", "codex": "a"}}));
        r.screen.show_uploaded("codex"); // not the active view any more
        assert!(r.display.calls().is_empty());
        r.screen.show_uploaded("claude");
        assert_eq!(r.display.calls(), ["show claude-usage-b.gif"]);
        r.display.failing_with(Some(DeviceError::Offline("x".into())));
        r.screen.show_uploaded("claude");
        assert!(r.screen.shown_failed());
    }

    #[test]
    fn what_is_remembered_and_the_old_file_format() {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({"claude": "a", "codex": "b", "ip": "10.0.0.9"})); // older files: the slot map itself
        assert_eq!((r.screen.slot_of("claude").as_deref(), r.screen.slot_of("codex").as_deref()), (Some("a"), Some("b")));
        assert_eq!(r.screen.ip(), "10.0.0.5", "--ip wins over the remembered address");
        assert!(r.screen.is_outdated("claude") && r.screen.is_outdated("codex"), "drawn by other code: refreshed in the background");
        let snap = r.screen.snapshot();
        assert_eq!(snap["render"], Value::Null, "not claimed while images are out of date");
        let current = json!({"slots": {"claude": "a"}, "ip": "10.0.0.7", "render": "test-render"});
        let r2 = rig(None);
        r2.screen.restore(&current);
        assert!(r2.screen.outdated_keys().is_empty());
        assert_eq!(Value::Object(r2.screen.snapshot()), current);
    }

    #[test]
    fn a_delivery_to_a_screen_that_is_there_also_keeps_the_picture() {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({}));
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Ok);
        let (view, _, gif) = r.screen.local.latest().unwrap();
        assert_eq!((view.as_str(), gif.as_slice()), ("claude", b"GIF89a".as_slice()));
    }

    #[test]
    fn with_no_address_the_picture_is_still_drawn_for_the_simulated_screen() {
        let r = rig(None);
        r.screen.restore(&json!({}));
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Offline);
        assert!(r.display.calls().is_empty(), "nothing goes to a screen that isn't there");
        assert!(r.screen.local.latest().is_some());
        assert_eq!(r.renderer.rendered.lk().len(), 1);
    }

    #[test]
    fn while_the_device_is_unreachable_a_retry_does_not_redraw_the_same_picture() {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({"slots": {"claude": "a"}}));
        r.display.failing_with(Some(DeviceError::Offline("no route".into())));
        let reading = usage("Claude");
        for _ in 0..4 {
            // (the worker retries every few seconds with the very same reading)
            assert_eq!(r.screen.deliver("claude", reading.clone()), Outcome::Offline);
        }
        assert_eq!(r.renderer.rendered.lk().len(), 1);
        let Data::One(mut newer) = usage("Claude") else { unreachable!() };
        newer.working = true; // an agent started: the same numbers now carry a "Working" tag
        assert_eq!(r.screen.deliver("claude", Data::One(newer)), Outcome::Offline);
        assert_eq!(r.renderer.rendered.lk().len(), 2, "a different picture is drawn");
    }

    #[test]
    fn a_failure_to_draw_for_the_simulated_screen_never_breaks_the_loop() {
        let r = rig(None);
        r.screen.restore(&json!({}));
        r.renderer.fail.store(true, Ordering::SeqCst);
        assert_eq!(r.screen.deliver("claude", usage("Claude")), Outcome::Offline);
        assert!(r.screen.local.latest().is_none());
    }

    #[test]
    fn error_screens_are_remembered_for_providers_only() {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({}));
        let err = |title: &str| Data::Error(ErrorScreen { title: title.into(), message: "not signed in".into(), now: chrono::Local::now(), signin: true });
        r.screen.deliver("claude", err("Claude"));
        assert_eq!(r.screen.error_shown("claude").as_deref(), Some("not signed in"));
        r.screen.clear_error_shown("claude");
        assert!(r.screen.error_shown("claude").is_none());
    }
}
