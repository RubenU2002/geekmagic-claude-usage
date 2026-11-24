//! The ports: what the application needs from the outside world, as traits.
//!
//! The application (`crate::application`) is written against these and nothing else, so it knows no AI tool, no screen
//! hardware, no drawing code and no operating system. Each adapter (`crate::adapters`) implements one of them; the
//! composition root (`crate::bootstrap`) is the only place that picks which implementation to use.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use serde_json::{Map, Value};

use super::activity::Summary;
use super::errors::{DeviceError, UsageError};
use super::model::{Data, Usage};
use super::sessions::SessionState;

/// Whether an agent is working or waiting for you right now, and every session seen.
#[derive(Debug, Clone, Default)]
pub struct AgentState {
    pub working: bool,
    pub waiting: bool,
    pub waiting_for: Option<String>,
    pub since: Option<f64>,
    pub last: Option<f64>,
    pub project: Option<String>,
    pub sessions: Vec<SessionState>,
}

/// An AI tool (Claude, Codex...). Everything that is specific to one lives behind this trait, so supporting another is
/// one new type and an entry in the composition root.
pub trait Provider: Send + Sync {
    /// "claude": what is stored on disk and used as a file name.
    fn key(&self) -> &str;
    /// "Claude": what the screens and notifications call it.
    fn title(&self) -> &str;
    /// The arguments of its CLI that start the sign-in.
    fn login_args(&self) -> &[&str];
    /// What to tell someone whose CLI isn't installed.
    fn install_hint(&self) -> &str;
    /// Its current usage. Fails with `sign_in_needed` when signing in again is what's needed. Never spends a token.
    fn fetch(&self) -> Result<Usage, UsageError>;
    /// Where its command-line tool is installed.
    fn find_cli(&self) -> Option<String>;
    /// Its activity counted from its local logs, None without logs.
    fn local_stats(&self, now_epoch: f64) -> Option<Summary>;
    /// Whether its agent is working or waiting for you right now.
    fn agent_state(&self, now_epoch: f64) -> AgentState;
}

/// One GeekMagic screen. Every method fails with `DeviceError::Offline` when it doesn't answer properly; `upload` fails
/// with `Cancelled` when `cancel` is set mid-transfer.
pub trait Display: Send + Sync {
    fn ip(&self) -> &str;
    fn upload(&self, image: &[u8], filename: &str, content_type: &str, cancel: Option<&AtomicBool>) -> Result<(), DeviceError>;
    fn list_images(&self) -> Result<HashSet<String>, DeviceError>;
    fn delete_image(&self, filename: &str) -> Result<(), DeviceError>;
    /// The stored image file, as it is on the device (what the screen is showing, when it is pinned).
    fn download_image(&self, filename: &str) -> Result<Vec<u8>, DeviceError>;
    fn show_image(&self, filename: &str) -> Result<(), DeviceError>;
    fn set_brightness(&self, level: f64) -> Result<(), DeviceError>;
    fn set_night_mode(&self, start_hour: i64, end_hour: i64, night_level: f64, day_level: f64, enabled: bool) -> Result<(), DeviceError>;
}

pub trait DeviceLocator: Send + Sync {
    /// Does a GeekMagic screen answer at this address?
    fn probe(&self, host: &str, timeout: f64) -> bool;
    /// Look for it on the local network (`prefer` is tried first).
    fn find_device(&self, prefer: Option<&str>) -> Option<String>;
    /// The networks this computer is on, as "192.168.1." prefixes, the main one first.
    fn networks(&self) -> Vec<String>;
}

/// What a Renderer returns: the GIF to upload, and which animations it picked from its rotation, to be remembered
/// (`Renderer::confirm`) only once the upload has gone through.
#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub gif: Vec<u8>,
    /// What each panel played, in order (for the log).
    pub animations: Vec<String>,
    /// (provider title, animation).
    pub picks: Vec<(String, String)>,
}

pub trait Renderer: Send + Sync {
    /// Identifies the drawing code: a stored image drawn by another version is redrawn.
    fn version(&self) -> &str;
    /// The screen for `key`: a provider's key (`data` is its Usage, or an ErrorScreen), or one of the views of two.
    fn render(&self, key: &str, data: &Data, animation: &str) -> Result<Rendered, String>;
    /// A still picture (PNG) of a GIF this renderer drew.
    fn to_png(&self, gif: &[u8]) -> Result<Vec<u8>, String>;
    /// The upload went through: remember what was picked, so it doesn't come back too soon.
    fn confirm(&self, rendered: &Rendered);
    fn restore(&self, saved: &Value);
    fn snapshot(&self) -> Map<String, Value>;
}

pub trait Notifier: Send + Sync {
    /// Show a notification. Fails if it couldn't.
    fn send(&self, title: &str, message: &str) -> Result<(), String>;
}

pub trait StateStore: Send + Sync {
    /// What was saved, or an empty map if there's nothing (or nothing readable).
    fn load(&self) -> Map<String, Value>;
    fn save(&self, state: &Map<String, Value>);
    fn path(&self) -> String;
}

pub trait LockSensor: Send + Sync {
    fn is_locked(&self) -> bool;
}

pub trait TerminalLauncher: Send + Sync {
    /// Run `argv` in a new terminal window that stays open: "started", or "failed".
    fn launch(&self, argv: &[String], title: &str) -> String;
}

/// The tray icon, from the application's side.
pub trait Ui: Send + Sync {
    fn set_title(&self, title: &str);
    /// Show the icon of a view (a provider's key, or one of the views of two).
    fn set_icon(&self, key: &str);
    fn refresh_menu(&self);
    /// Ask where to save a picture (a .png, or a .gif to keep it animated): the path chosen, None if cancelled.
    fn ask_save_path(&self, suggested_name: &str) -> Option<String>;
    /// Open the window that simulates the screen (or bring it to the front if it is open).
    fn show_simulator(&self) {}
    /// Open the simulated screen's window, or close it if it is open.
    fn toggle_simulator(&self) {}
    /// Show the icon and handle its clicks; returns when it is stopped.
    fn run(&self);
    fn stop(&self);
}
