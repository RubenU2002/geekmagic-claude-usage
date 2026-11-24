//! The tray app's controller: builds the use cases from the ports it is given, wires them together, and handles what the
//! icon's clicks, its menu and the webhooks ask for.
//!
//! It knows no desktop toolkit, no drawing code and no device: everything outside the application arrives through a port
//! (see `crate::core::ports`), so it can be driven by anything that implements `Ui` (the tray icon, or a test).

use std::sync::{Arc, OnceLock};

use super::activity::ActivityCounter;
use super::agents::AgentMonitor;
use super::backlight::Backlight;
use super::capture::ScreenCapture;
use super::notifications::Notifications;
use super::persistence::{Part, Persistence, RendererPart};
use super::power::Power;
use super::scheduler::Scheduler;
use super::screen::{Screen, ScreenHooks};
use super::signin::SignInCoordinator;
use super::usage::{AlertTracker, UsageHistory, UsageService};
use super::viewstate::ViewState;
use super::{config, Callback, Notify};
use crate::core::ports::{DeviceLocator, Display, LockSensor, Notifier, Renderer, StateStore, TerminalLauncher, Ui};
use crate::core::registry::ProviderRegistry;
use crate::core::views::view_name;
use crate::util::{in_background, mono, truncate_chars, Event};

/// How the app was asked to run.
#[derive(Debug, Clone)]
pub struct Settings {
    pub ip: Option<String>,
    /// The slow fallback refresh in seconds (0: only webhooks, clicks and agent changes start a cycle).
    pub interval: f64,
    pub animation: String,
    pub provider: Option<String>,
    pub night_hours: Option<(i64, i64)>,
    pub night_level: Option<i64>,
    /// Count the stats in a thread (the first count of a month of logs takes seconds); tests turn it off.
    pub background_stats: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ip: None,
            interval: config::FALLBACK_INTERVAL,
            animation: "auto".into(),
            provider: None,
            night_hours: None,
            night_level: None,
            background_stats: true,
        }
    }
}

/// Everything outside the application the controller needs.
pub struct Ports {
    pub providers: Arc<ProviderRegistry>,
    pub store: Arc<dyn StateStore>,
    /// (ip) -> the screen at that address.
    pub display_factory: Arc<dyn Fn(&str) -> Arc<dyn Display> + Send + Sync>,
    pub locator: Arc<dyn DeviceLocator>,
    pub renderer: Arc<dyn Renderer>,
    pub notifier: Arc<dyn Notifier>,
    pub lock_sensor: Arc<dyn LockSensor>,
    pub terminal: Arc<dyn TerminalLauncher>,
    pub ui: Arc<dyn Ui>,
}

/// Something a webhook tells the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookEvent {
    /// Read the usage again (of one provider, or of all).
    Refresh { provider: Option<String> },
    /// A provider's tool reported something: a Claude Code hook (`Stop`, `UserPromptSubmit`...) or its Codex equivalent.
    Hook { provider: String, event: String },
}

/// Events that only mean "an agent is doing something": the agent view changes, the usage doesn't yet. Anything else a
/// hook reports (a turn ended, a session ended...) means the usage has changed and is read again.
const ACTIVITY_ONLY: [&str; 5] = ["UserPromptSubmit", "SessionStart", "PreToolUse", "Notification", "PermissionRequest"];

pub struct TrayController {
    pub providers: Arc<ProviderRegistry>,
    pub ui: Arc<dyn Ui>,
    pub wake: Arc<Event>,
    pub stop: Arc<Event>,
    pub notifications: Arc<Notifications>,
    pub view: Arc<ViewState>,
    pub persistence: Arc<Persistence>,
    pub usage: Arc<UsageService>,
    pub agents: Arc<AgentMonitor>,
    pub activity: Arc<ActivityCounter>,
    pub screen: Arc<Screen>,
    pub capture: Arc<ScreenCapture>,
    pub backlight: Arc<Backlight>,
    pub power: Arc<Power>,
    pub signin: Arc<SignInCoordinator>,
    pub scheduler: Arc<Scheduler>,
}

type Tooltip = Arc<dyn Fn() -> String + Send + Sync>;

impl TrayController {
    pub fn new(ports: Ports, settings: Settings) -> Arc<Self> {
        let Ports { providers, store, display_factory, locator, renderer, notifier, lock_sensor, terminal, ui } = ports;
        let wake = Arc::new(Event::new());
        let stop = Arc::new(Event::new());
        let notifications = Arc::new(Notifications::new(notifier));
        let notify: Notify = {
            let n = notifications.clone();
            Arc::new(move |title, message| n.send(title, message))
        };
        let view = Arc::new(ViewState::new(providers.keys(), settings.provider.clone()));
        let persistence = Arc::new(Persistence::new(store));
        let save: Callback = {
            let p = persistence.clone();
            Arc::new(move || p.save())
        };

        // the tooltip needs most of the parts, which need the tooltip: it is filled in at the end
        let tooltip_cell: Arc<OnceLock<Tooltip>> = Arc::new(OnceLock::new());
        let tooltip: Tooltip = {
            let cell = tooltip_cell.clone();
            Arc::new(move || cell.get().map(|f| f()).unwrap_or_default())
        };
        let set_title: Arc<dyn Fn(&str) + Send + Sync> = {
            let ui = ui.clone();
            Arc::new(move |title| ui.set_title(title))
        };
        let refresh_menu: Callback = {
            let ui = ui.clone();
            Arc::new(move || ui.refresh_menu())
        };

        let history = Arc::new(UsageHistory::default());
        let alert_tracker = Arc::new(AlertTracker::new(providers.clone(), notify.clone(), save.clone()));
        let usage = Arc::new(UsageService::new(providers.clone(), history.clone(), alert_tracker.clone()));
        let agents = Arc::new(AgentMonitor::new(
            providers.clone(),
            notify.clone(),
            {
                // an agent started or stopped: redraw with (or without) the working indicator, from the readings we have
                let (set_title, tooltip, wake) = (set_title.clone(), tooltip.clone(), wake.clone());
                Arc::new(move || {
                    set_title(&tooltip());
                    wake.set();
                })
            },
            stop.clone(),
            {
                let n = notifications.clone();
                Arc::new(move || n.agent_events())
            },
        ));
        let activity = ActivityCounter::new(providers.clone(), { let w = wake.clone(); Arc::new(move || w.set()) }, settings.background_stats);
        let screen = Arc::new(Screen::new(
            settings.ip.clone(),
            &settings.animation,
            providers.clone(),
            renderer.clone(),
            locator,
            ScreenHooks {
                display_factory,
                active_key: { let v = view.clone(); Arc::new(move || v.key()) },
                flag: { let a = agents.clone(); Arc::new(move |key, data| a.flag(key, data)) },
                notify: notify.clone(),
                save: save.clone(),
                set_title: set_title.clone(),
                tooltip: tooltip.clone(),
                refresh_menu: refresh_menu.clone(),
            },
            wake.clone(),
        ));
        let capture = ScreenCapture::new(screen.clone(), renderer.clone(), { let ui = ui.clone(); Arc::new(move |name| ui.ask_save_path(name)) }, notify.clone());
        let backlight = Backlight::new(screen.clone(), notify.clone(), save.clone(), refresh_menu.clone(), settings.night_hours, settings.night_level);
        let power = Arc::new(Power::new(
            lock_sensor,
            {
                let (set_title, tooltip, refresh_menu) = (set_title.clone(), tooltip.clone(), refresh_menu.clone());
                Arc::new(move || {
                    set_title(&tooltip());
                    refresh_menu();
                })
            },
            {
                let (backlight, screen, view) = (backlight.clone(), screen.clone(), view.clone());
                Arc::new(move |locked| {
                    backlight.on_lock_change(locked);
                    if !locked {
                        let (screen, key) = (screen.clone(), view.key());
                        in_background(move || screen.show_uploaded(&key)); // put the view back
                    }
                })
            },
            wake.clone(),
            stop.clone(),
            save.clone(),
        ));
        let signin = Arc::new(SignInCoordinator::new(providers.clone(), usage.clone(), screen.clone(), notify.clone(), wake.clone(), terminal));
        usage.set_on_read({
            let signin = signin.clone();
            move |provider| signin.on_read(provider)
        });
        let scheduler = Arc::new(Scheduler::new(
            providers.clone(),
            settings.interval,
            usage.clone(),
            screen.clone(),
            view.clone(),
            power.clone(),
            agents.clone(),
            activity.clone(),
            signin.clone(),
            wake.clone(),
            stop.clone(),
            tooltip.clone(),
            set_title.clone(),
        ));
        usage.set_on_failure({
            let (signin, scheduler, view, set_title) = (signin.clone(), scheduler.clone(), view.clone(), set_title.clone());
            move |provider, error| {
                signin.on_failure(provider, error);
                if provider == view.provider() {
                    set_title(&truncate_chars(&format!("GeekMagic: {error}"), 120));
                }
                scheduler.mark_stale(provider);
            }
        });

        let parts: Vec<Arc<dyn Part>> = vec![
            view.clone(),
            notifications.clone(),
            Arc::new(RendererPart(renderer.clone())),
            alert_tracker,
            history,
            screen.clone(),
            backlight.clone(),
            power.clone(),
        ];
        for part in parts {
            persistence.register(part);
        }
        persistence.load();

        let controller = Arc::new(Self {
            providers,
            ui,
            wake,
            stop,
            notifications,
            view,
            persistence,
            usage,
            agents,
            activity,
            screen,
            capture,
            backlight,
            power,
            signin,
            scheduler,
        });
        let _ = tooltip_cell.set({
            let c = controller.clone();
            Arc::new(move || c.tooltip_text())
        });
        controller
    }

    // --- what the icon says ---------------------------------------------------------------------------------

    fn tooltip_text(&self) -> String {
        let mut view = match self.view.mode() {
            Some(mode) => view_name(&mode).to_string(),
            None => format!("mostrando {}", self.providers.title_of(&self.view.provider())),
        };
        let notes = self.agents.tooltip_notes();
        if !notes.is_empty() {
            view += &format!(" · {}", notes.join(" y "));
        }
        view += &format!(" · {}", self.screen.status_note());
        if self.power.paused() {
            format!("GeekMagic: {view} (en pausa)")
        } else if self.power.pause_on_lock() && self.power.locked() {
            format!("GeekMagic: {view} (en pausa: PC bloqueado)")
        } else {
            format!("GeekMagic: {view}")
        }
    }

    pub fn tooltip(&self) -> String {
        self.tooltip_text()
    }

    // --- clicks and menu commands ---------------------------------------------------------------------------

    /// Common to every switch: pin the view's last image at once and refresh it in the background.
    fn switched(&self, key: &str) {
        self.persistence.save(); // so a restart or reconnection comes back to this view
        self.usage.reset_backoff(); // you're looking at it: try again even if it has been failing
        self.ui.set_title(&self.tooltip());
        self.ui.refresh_menu();
        self.screen.note_click(); // abort any upload in flight: the device serves one request at a time
        let (screen, shown) = (self.screen.clone(), key.to_string());
        in_background(move || screen.show_uploaded(&shown));
        if mono() - self.screen.last_upload(key) > config::STALE_AFTER {
            self.wake.set(); // refresh the image we just switched to, once clicking settles
        }
    }

    pub fn select(&self, provider: &str) {
        log::info!("switch -> {provider}");
        self.view.show_provider(provider);
        self.ui.set_icon(provider);
        self.switched(provider);
        let (signin, provider) = (self.signin.clone(), provider.to_string());
        in_background(move || signin.ask(&provider, false)); // (only does anything if it's signed out)
    }

    /// Left-click on the icon. With the screen there, on to the next provider (see `toggle`). Without it the tray is the
    /// screen: the window that simulates it opens (and a second click closes it); a click inside that window is what goes on
    /// to the next provider.
    pub fn tray_click(&self) {
        if !self.screen.ip().is_empty() && !self.screen.is_offline() {
            self.toggle();
        } else {
            self.ui.toggle_simulator();
        }
    }

    /// Menu: the simulated screen's window, whether or not the real one is there.
    pub fn open_simulator(&self) {
        self.ui.show_simulator();
    }

    /// Left-click: on to the next provider. From a view of both it goes back to the provider you were on.
    pub fn toggle(&self) {
        if self.view.mode().is_some() {
            self.select(&self.view.provider());
            return;
        }
        let keys = self.providers.keys();
        let at = keys.iter().position(|k| *k == self.view.provider()).unwrap_or(0);
        self.select(&keys[(at + 1) % keys.len()]);
    }

    /// Menu: a view of both providers at once. Turning it off returns to the last single provider.
    pub fn toggle_mode(&self, mode: &str) {
        if self.view.mode().as_deref() == Some(mode) {
            self.select(&self.view.provider());
            return;
        }
        log::info!("switch -> {mode} view");
        self.view.set_mode(Some(mode));
        self.ui.set_icon(mode);
        self.switched(mode);
    }

    pub fn sign_in(&self, provider: &str) {
        let (signin, provider) = (self.signin.clone(), provider.to_string());
        in_background(move || signin.ask(&provider, true));
    }

    pub fn toggle_notifications(&self) {
        self.notifications.set_enabled(!self.notifications.enabled());
        log::info!("notifications {}", if self.notifications.enabled() { "on" } else { "off" });
        self.persistence.save();
        self.ui.refresh_menu();
    }

    pub fn toggle_agent_events(&self) {
        self.notifications.set_agent_events(!self.notifications.agent_events());
        log::info!("agent-finished notifications {}", if self.notifications.agent_events() { "on" } else { "off" });
        self.persistence.save();
        self.ui.refresh_menu();
    }

    pub fn refresh_now(&self) {
        self.usage.reset_backoff();
        for provider in self.providers.keys() {
            self.usage.forget_fetch(&provider);
        }
        self.wake.set();
    }

    // --- webhooks -------------------------------------------------------------------------------------------

    /// A webhook arrived: do what it asks right away. It only wakes the worker (and the agents' watcher); the work is
    /// done by the same cycle a click starts, so many webhooks in a row make one read, not many.
    pub fn on_webhook(&self, event: WebhookEvent) {
        match event {
            WebhookEvent::Refresh { provider } => {
                log::info!("webhook: refresh {}", provider.as_deref().unwrap_or("all"));
                self.agents.nudge();
                self.refresh_providers(provider.as_deref());
            }
            WebhookEvent::Hook { provider, event } => {
                log::info!("webhook: {provider} {event}");
                self.agents.nudge(); // is something running, or waiting for you, now?
                if !ACTIVITY_ONLY.contains(&event.as_str()) {
                    self.refresh_providers(Some(&provider));
                }
            }
        }
    }

    fn refresh_providers(&self, provider: Option<&str>) {
        self.usage.reset_backoff();
        match provider.filter(|p| self.providers.contains(p)) {
            Some(p) => self.usage.forget_fetch(p),
            None => self.providers.keys().iter().for_each(|p| self.usage.forget_fetch(p)),
        }
        self.wake.set();
    }

    pub fn quit(&self) {
        self.stop.set();
        self.wake.set();
        self.agents.nudge();
        self.ui.stop();
    }

    pub fn run(self: &Arc<Self>) {
        for work in [
            Box::new({ let c = self.clone(); move || c.scheduler.run() }) as Box<dyn FnOnce() + Send>,
            Box::new({ let c = self.clone(); move || c.power.watch_lock() }),
            Box::new({ let c = self.clone(); move || c.agents.watch() }),
        ] {
            let _ = std::thread::Builder::new().stack_size(1024 * 1024).spawn(work);
        }
        self.ui.run();
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::application::persistence::tests::MemoryStore;
    use crate::application::testing::*;
    use crate::application::usage::tests::{registry, Scripted};
    use crate::application::notifications::tests::Recorder;
    use crate::util::LockExt;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct FakeUi {
        pub titles: Mutex<Vec<String>>,
        pub icons: Mutex<Vec<String>>,
        pub menus: AtomicUsize,
        pub stopped: AtomicBool,
        /// What the application asked of the simulated screen's window: "show" / "toggle".
        pub simulator: Mutex<Vec<String>>,
    }

    impl Ui for FakeUi {
        fn set_title(&self, title: &str) {
            self.titles.lk().push(title.into());
        }
        fn set_icon(&self, key: &str) {
            self.icons.lk().push(key.into());
        }
        fn refresh_menu(&self) {
            self.menus.fetch_add(1, Ordering::SeqCst);
        }
        fn ask_save_path(&self, _: &str) -> Option<String> {
            None
        }
        fn show_simulator(&self) {
            self.simulator.lk().push("show".into());
        }
        fn toggle_simulator(&self) {
            self.simulator.lk().push("toggle".into());
        }
        fn run(&self) {
            while !self.stopped.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        fn stop(&self) {
            self.stopped.store(true, Ordering::SeqCst);
        }
    }

    struct Unlocked;
    impl LockSensor for Unlocked {
        fn is_locked(&self) -> bool {
            false
        }
    }

    struct NoTerminal;
    impl TerminalLauncher for NoTerminal {
        fn launch(&self, _: &[String], _: &str) -> String {
            "failed".into()
        }
    }

    pub struct App {
        pub app: Arc<TrayController>,
        pub store: Arc<MemoryStore>,
        pub ui: Arc<FakeUi>,
        pub display: Arc<FakeDisplay>,
        pub renderer: Arc<FakeRenderer>,
        pub claude: Arc<Scripted>,
        pub codex: Arc<Scripted>,
        pub notifier: Arc<Recorder>,
    }

    pub fn app(settings: Settings, saved: Value) -> App {
        let claude = Scripted::new("claude", "Claude", vec![Ok((30.0, 10.0))]);
        let codex = Scripted::new("codex", "Codex", vec![Ok((20.0, 5.0))]);
        let store = Arc::new(MemoryStore::default());
        store.state.lk().extend(saved.as_object().cloned().unwrap_or_default());
        let (ui, display, renderer, notifier) = (Arc::new(FakeUi::default()), FakeDisplay::new("10.0.0.5"), Arc::new(FakeRenderer::default()), Arc::new(Recorder::default()));
        let d = display.clone();
        let ports = Ports {
            providers: registry(vec![claude.clone(), codex.clone()]),
            store: store.clone(),
            display_factory: Arc::new(move |_| d.clone() as Arc<dyn Display>),
            locator: FakeLocator::answering(&["10.0.0.5"]),
            renderer: renderer.clone(),
            notifier: notifier.clone(),
            lock_sensor: Arc::new(Unlocked),
            terminal: Arc::new(NoTerminal),
            ui: ui.clone(),
        };
        let app = TrayController::new(ports, Settings { ip: Some("10.0.0.5".into()), background_stats: false, ..settings });
        App { app, store, ui, display, renderer, claude, codex, notifier }
    }

    fn quick() -> Settings {
        Settings::default()
    }

    #[test]
    fn the_view_comes_back_as_it_was_left() {
        let a = app(quick(), json!({"view": "split", "provider": "codex"}));
        assert_eq!((a.app.view.mode().as_deref(), a.app.view.provider().as_str()), (Some("split"), "codex"));
        assert_eq!(a.app.tooltip(), "GeekMagic: vista dividida · pantalla 10.0.0.5");
        let b = app(quick(), json!({"view": "claude"}));
        assert_eq!(b.app.tooltip(), "GeekMagic: mostrando Claude · pantalla 10.0.0.5");
    }

    #[test]
    fn left_click_cycles_providers_and_leaves_a_view_of_both() {
        let a = app(quick(), json!({}));
        a.app.toggle();
        assert_eq!(a.app.view.key(), "codex");
        a.app.toggle();
        assert_eq!(a.app.view.key(), "claude", "round the list");
        a.app.toggle_mode("stats");
        assert_eq!((a.app.view.key().as_str(), a.app.view.provider().as_str()), ("stats", "claude"));
        a.app.toggle();
        assert_eq!(a.app.view.key(), "claude", "from a view of both: back to the provider you were on");
        a.app.toggle_mode("hours");
        a.app.toggle_mode("hours");
        assert_eq!(a.app.view.key(), "claude", "the same menu item again turns it off");
        assert_eq!(*a.ui.icons.lk(), ["codex", "claude", "stats", "claude", "hours", "claude"]);
        assert_eq!(a.store.state.lk()["view"], "claude", "every switch is remembered");
    }

    #[test]
    fn with_the_screen_there_a_click_goes_on_to_the_next_provider() {
        let a = app(quick(), json!({}));
        a.app.tray_click();
        assert_eq!(a.app.view.key(), "codex");
        assert!(a.ui.simulator.lk().is_empty());
    }

    #[test]
    fn without_the_screen_the_click_opens_the_simulated_one_instead() {
        let a = app(quick(), json!({}));
        a.app.screen.mark_offline("test");
        a.app.tray_click();
        assert_eq!((a.ui.simulator.lk().clone(), a.app.view.key()), (vec!["toggle".to_string()], "claude".to_string()), "the view didn't change");
        a.app.screen.came_back();
        a.app.tray_click();
        assert_eq!(a.app.view.key(), "codex");
        let b = app(quick(), json!({}));
        b.app.screen.set_ip("", false); // the address is lost: nothing to talk to
        b.app.tray_click();
        assert_eq!(*b.ui.simulator.lk(), ["toggle"], "no address at all: also the simulated screen");
        b.app.open_simulator();
        assert_eq!(*b.ui.simulator.lk(), ["toggle", "show"]);
    }

    #[test]
    fn a_switch_pins_the_last_image_at_once_and_stops_uploads_for_a_moment() {
        let a = app(quick(), json!({"slots": {"codex": "b"}, "render": "test-render"}));
        a.app.toggle();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(a.display.calls().contains(&"show codex-usage-b.gif".to_string()), "{:?}", a.display.calls());
        assert!(mono() - a.app.screen.last_click() < 2.0);
        assert!(a.app.wake.is_set(), "no upload of it yet: refreshed once clicking settles");
    }

    #[test]
    fn webhooks_wake_the_worker_and_say_which_reading_is_stale() {
        let a = app(quick(), json!({}));
        a.app.usage.fetch("claude");
        a.app.usage.fetch("codex");
        assert!(a.app.usage.last_fetch("codex") > 0.0);
        a.app.wake.clear();
        a.app.on_webhook(WebhookEvent::Hook { provider: "codex".into(), event: "Stop".into() });
        assert!(a.app.wake.is_set());
        assert_eq!(a.app.usage.last_fetch("codex"), 0.0, "its reading is due again");
        assert!(a.app.usage.last_fetch("claude") > 0.0, "the other one is left alone");
        a.app.usage.fetch("codex");
        a.app.wake.clear();
        a.app.on_webhook(WebhookEvent::Hook { provider: "codex".into(), event: "UserPromptSubmit".into() });
        assert!(!a.app.wake.is_set() && a.app.usage.last_fetch("codex") > 0.0, "a prompt starting changes what the agent is doing, not the usage");
        a.app.on_webhook(WebhookEvent::Refresh { provider: None });
        assert!(a.app.wake.is_set() && a.app.usage.last_fetch("claude") == 0.0 && a.app.usage.last_fetch("codex") == 0.0);
        a.app.usage.fetch("claude");
        a.app.on_webhook(WebhookEvent::Refresh { provider: Some("nope".into()) });
        assert_eq!(a.app.usage.last_fetch("claude"), 0.0, "an unknown provider means all of them");
    }

    #[test]
    fn the_switches_are_remembered_and_the_menu_is_told() {
        let a = app(quick(), json!({}));
        let before = a.ui.menus.load(Ordering::SeqCst);
        a.app.toggle_notifications();
        a.app.toggle_agent_events();
        let saved = a.store.state.lk().clone();
        assert_eq!((saved["notify"].clone(), saved["notify_done"].clone()), (json!(false), json!(false)));
        assert_eq!(a.ui.menus.load(Ordering::SeqCst), before + 2);
        a.app.notifications.send("x", "muted");
        assert!(a.notifier.sent.lk().is_empty());
    }

    #[test]
    fn the_whole_app_runs_end_to_end_on_events_with_no_timer() {
        let a = app(Settings { interval: 0.0, ..quick() }, json!({}));
        let runner = a.app.clone();
        let t = std::thread::spawn(move || runner.run());
        let wait_for = |what: &str, cond: &dyn Fn() -> bool| {
            let started = std::time::Instant::now();
            while !cond() {
                assert!(started.elapsed() < std::time::Duration::from_secs(15), "timed out waiting for {what}: {:?}", a.display.calls());
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        wait_for("the first picture", &|| a.display.calls().iter().any(|c| c == "show claude-usage-a.gif"));
        wait_for("the other provider's picture", &|| a.display.calls().iter().any(|c| c == "upload codex-usage-a.gif"));
        assert_eq!(a.claude.calls.load(Ordering::SeqCst), 1);
        std::thread::sleep(std::time::Duration::from_millis(600));
        assert_eq!(a.claude.calls.load(Ordering::SeqCst), 1, "no timer: nothing more happens by itself");
        a.app.on_webhook(WebhookEvent::Hook { provider: "claude".into(), event: "Stop".into() });
        wait_for("the webhook's read", &|| a.claude.calls.load(Ordering::SeqCst) == 2);
        wait_for("its upload", &|| a.display.calls().iter().any(|c| c == "upload claude-usage-b.gif"));
        a.app.quit();
        t.join().unwrap();
        assert!(a.ui.stopped.load(Ordering::SeqCst));
        assert!(!a.renderer.rendered.lk().is_empty());
        assert_eq!(a.store.state.lk()["ip"], "10.0.0.5");
    }
}
