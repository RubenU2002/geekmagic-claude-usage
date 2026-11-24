//! The Ui port, as a system-tray icon (Windows).
//!
//! The icon, its tooltip and its menu belong to the thread that runs the message loop (`run`). Everything the application
//! asks of the icon from other threads is queued and the loop is woken to do it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, PeekMessageW, PostQuitMessage, PostThreadMessageW, TranslateMessage, MSG, PM_NOREMOVE, WM_APP};

use super::icons::{icon_for, Rgba};
use super::menu::{self, Action, Item};
use super::simulator::SimulatorWindow;
use crate::application::controller::TrayController;
use crate::core::ports::Ui;
use crate::util::{in_background, LockExt};

enum Cmd {
    Title(String),
    Icon(String),
    Menu,
    Stop,
}

pub struct TrayUi {
    tx: Mutex<Sender<Cmd>>,
    rx: Mutex<Option<Receiver<Cmd>>>,
    thread: AtomicU32, // the message loop's thread, once it runs
    controller: OnceLock<Weak<TrayController>>,
    simulator: OnceLock<Arc<SimulatorWindow>>,
    log_path: PathBuf,
    last_folder: Mutex<PathBuf>,
    actions: Arc<Mutex<HashMap<String, Action>>>, // menu item id -> what it does
}

impl TrayUi {
    pub fn new(log_path: PathBuf) -> Arc<Self> {
        let (tx, rx) = channel();
        let home = crate::adapters::providers::executables::home_dir();
        let pictures = home.join("Pictures");
        Arc::new(Self {
            tx: Mutex::new(tx),
            rx: Mutex::new(Some(rx)),
            thread: AtomicU32::new(0),
            controller: OnceLock::new(),
            simulator: OnceLock::new(),
            log_path,
            last_folder: Mutex::new(if pictures.is_dir() { pictures } else { home }),
            actions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// The controller whose state the menu shows and whose commands it runs.
    pub fn attach(&self, controller: &Arc<TrayController>) {
        let weak = Arc::downgrade(controller);
        let _ = self.controller.set(weak.clone());
        // the simulated screen shows the pictures the app draws for the device; a click in it goes on to the next provider
        let (latest, click) = (weak.clone(), weak);
        let _ = self.simulator.set(SimulatorWindow::new(
            Arc::new(move || latest.upgrade().and_then(|app| app.screen.local.latest())),
            Arc::new(move || {
                if let Some(app) = click.upgrade() {
                    app.toggle();
                }
            }),
            Arc::new(|| "GeekMagic (simulada)".to_string()),
        ));
    }

    fn controller(&self) -> Option<Arc<TrayController>> {
        self.controller.get().and_then(Weak::upgrade)
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.lk().send(cmd);
        let thread = self.thread.load(Ordering::SeqCst);
        if thread != 0 {
            // SAFETY: posting a plain message to a thread id; failure (the thread is gone) is harmless.
            unsafe {
                PostThreadMessageW(thread, WM_APP, 0, 0);
            }
        }
    }

    /// A real menu for the application's state now, and the table of what its items do.
    fn build_menu(&self) -> Option<Menu> {
        let app = self.controller()?;
        let items = menu::build(&app);
        let mut actions = HashMap::new();
        let mut next = 0usize;
        let menu = Menu::new();
        for item in realise(&items, &mut actions, &mut next) {
            let _ = menu.append(item.as_ref());
        }
        *self.actions.lk() = actions;
        Some(menu)
    }

    fn icon(&self, key: &str) -> Option<Icon> {
        let titles = self.controller()?.providers.titles();
        to_icon(&icon_for(key, &titles))
    }

    fn run_command(&self, tray: &TrayIcon, cmd: Cmd) {
        match cmd {
            Cmd::Title(title) => {
                let _ = tray.set_tooltip(Some(title.chars().take(127).collect::<String>()));
            }
            Cmd::Icon(key) => {
                if let Some(icon) = self.icon(&key) {
                    let _ = tray.set_icon(Some(icon));
                }
            }
            Cmd::Menu => {
                if let Some(menu) = self.build_menu() {
                    tray.set_menu(Some(Box::new(menu)));
                }
            }
            // SAFETY: asks this thread's message loop to end.
            Cmd::Stop => unsafe { PostQuitMessage(0) },
        }
    }
}

fn to_icon(rgba: &Rgba) -> Option<Icon> {
    Icon::from_rgba(rgba.px.clone(), rgba.w as u32, rgba.h as u32).ok()
}

/// Turn the described menu into the toolkit's items, giving each command an id and remembering what it does.
fn realise(items: &[Item], actions: &mut HashMap<String, Action>, next: &mut usize) -> Vec<Box<dyn IsMenuItem>> {
    let mut out: Vec<Box<dyn IsMenuItem>> = Vec::new();
    for item in items {
        match item {
            Item::Label(text) => out.push(Box::new(MenuItem::new(text, false, None))),
            Item::Separator => out.push(Box::new(PredefinedMenuItem::separator())),
            Item::Command { text, checked, action } => {
                *next += 1;
                let id = format!("item{next}");
                actions.insert(id.clone(), action.clone());
                match checked {
                    Some(on) => out.push(Box::new(CheckMenuItem::with_id(id, text, true, *on, None))),
                    None => out.push(Box::new(MenuItem::with_id(id, text, true, None))),
                }
            }
            Item::Submenu { text, items } => {
                let sub = Submenu::new(text, true);
                for child in realise(items, actions, next) {
                    let _ = sub.append(child.as_ref());
                }
                out.push(Box::new(sub));
            }
        }
    }
    out
}

impl Ui for TrayUi {
    fn set_title(&self, title: &str) {
        self.send(Cmd::Title(title.to_string()));
    }

    fn set_icon(&self, key: &str) {
        self.send(Cmd::Icon(key.to_string()));
    }

    fn refresh_menu(&self) {
        self.send(Cmd::Menu);
    }

    /// The system's own "save as" window, opened in the folder of the last picture (or Pictures).
    fn ask_save_path(&self, suggested_name: &str) -> Option<String> {
        let folder = {
            let last = self.last_folder.lk();
            if last.is_dir() { last.clone() } else { crate::adapters::providers::executables::home_dir() }
        };
        let chosen = super::dialog::ask_save_path(&folder, suggested_name)?;
        if let Some(parent) = std::path::Path::new(&chosen).parent() {
            *self.last_folder.lk() = parent.to_path_buf();
        }
        Some(chosen)
    }

    fn run(&self) {
        let Some(rx) = self.rx.lk().take() else { return };
        // SAFETY: querying this thread's id, and creating its message queue before anyone posts to it.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
            self.thread.store(GetCurrentThreadId(), Ordering::SeqCst);
        }
        let Some(app) = self.controller() else {
            log::error!("the tray icon has nothing to show: no controller attached");
            return;
        };
        let icon = self.icon(&app.view.key());
        let mut builder = TrayIconBuilder::new().with_tooltip(app.tooltip().chars().take(127).collect::<String>()).with_menu_on_left_click(false);
        if let Some(icon) = icon {
            builder = builder.with_icon(icon);
        }
        if let Some(menu) = self.build_menu() {
            builder = builder.with_menu(Box::new(menu));
        }
        let tray = match builder.build() {
            Ok(tray) => tray,
            Err(e) => {
                log::error!("could not create the tray icon: {e}");
                return;
            }
        };
        // choices and clicks arrive on this thread, from inside the message loop
        let me = app.clone();
        let actions = self.actions.clone();
        let ui_for_menu: Arc<TrayUiHandle> = Arc::new(TrayUiHandle { actions, controller: Arc::downgrade(&me), log_path: self.log_path.clone() });
        let menu_handle = ui_for_menu.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| menu_handle.choose(&event.id.0)));
        let click_app = Arc::downgrade(&app);
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                if let Some(app) = click_app.upgrade() {
                    in_background(move || app.tray_click());
                }
            }
        }));
        while let Ok(cmd) = rx.try_recv() {
            self.run_command(&tray, cmd);
        }
        // SAFETY: the standard message loop; `msg` is only read between calls.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == WM_APP && msg.hwnd.is_null() {
                    while let Ok(cmd) = rx.try_recv() {
                        self.run_command(&tray, cmd);
                    }
                    continue;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        drop(tray); // takes the icon off the tray
    }

    fn show_simulator(&self) {
        if let Some(window) = self.simulator.get() {
            window.show();
        }
    }

    fn toggle_simulator(&self) {
        if let Some(window) = self.simulator.get() {
            window.toggle();
        }
    }

    fn stop(&self) {
        if let Some(window) = self.simulator.get() {
            window.close();
        }
        self.send(Cmd::Stop);
    }
}

/// What the menu's event handler needs, apart from the icon (which stays on the message loop's thread).
struct TrayUiHandle {
    actions: Arc<Mutex<HashMap<String, Action>>>,
    controller: Weak<TrayController>,
    log_path: PathBuf,
}

impl TrayUiHandle {
    fn choose(&self, id: &str) {
        let Some(action) = self.actions.lk().get(id).cloned() else { return };
        let Some(app) = self.controller.upgrade() else { return };
        let log_path = self.log_path.clone();
        in_background(move || match action {
            Action::Select(key) => app.select(&key),
            Action::Mode(mode) => app.toggle_mode(&mode),
            Action::RefreshNow => app.refresh_now(),
            Action::Simulator => app.open_simulator(),
            Action::Capture => app.capture.take(),
            Action::TogglePause => app.power.toggle_pause(),
            Action::Brightness(level) => app.backlight.set_brightness(level),
            Action::ToggleNight => app.backlight.toggle_night(),
            Action::ToggleDimOnLock => app.backlight.toggle_dim_on_lock(),
            Action::SignIn(key) => app.sign_in(&key),
            Action::ToggleNotifications => app.toggle_notifications(),
            Action::ToggleAgentEvents => app.toggle_agent_events(),
            Action::TogglePauseOnLock => app.power.toggle_pause_on_lock(),
            Action::OpenLogs => {
                let _ = Command::new("notepad.exe").arg(&log_path).spawn();
            }
            Action::Quit => app.quit(),
        });
    }
}
