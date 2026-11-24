//! The composition root: the one place that knows which adapter implements each port, where the app keeps its files, and
//! how it logs. Everything is built here and handed in, so the application never reaches for a concrete type.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::adapters::device::client::GeekMagicDevice;
use crate::adapters::device::discovery::{FixedLocator, Locator};
use crate::adapters::providers::default_providers;
use crate::adapters::render::SoftRenderer;
use crate::adapters::store::JsonStateStore;
use crate::adapters::system::notifier::SystemNotifier;
use crate::adapters::system::session_lock::SessionLockSensor;
use crate::adapters::system::single_instance;
use crate::adapters::system::terminal::Terminal;
use crate::adapters::tray::headless::HeadlessUi;
use crate::adapters::webhook;
use crate::application::controller::{Ports, Settings, TrayController, WebhookEvent};
use crate::core::ports::Ui;
use crate::core::registry::ProviderRegistry;
use crate::util::LockExt;

/// Where the app keeps its files. The lock and the state file are the Python version's own (same names, same format), so the
/// two apps can't run at once and either picks up where the other left off; the log is separate.
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn log(&self) -> PathBuf {
        self.root.join("tray-rs.log")
    }
    pub fn lock(&self) -> PathBuf {
        self.root.join("tray.lock")
    }
    pub fn state(&self) -> PathBuf {
        self.root.join("tray_state.json")
    }
}

/// `--data-dir`, else `GEEKMAGIC_DATA_DIR`, else the project folder (the nearest folder above the program that holds
/// `tray.py`), else the program's own folder.
pub fn resolve_data_dir(given: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = given.or_else(|| std::env::var_os("GEEKMAGIC_DATA_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)) {
        return dir;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let here = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    here.ancestors().take(6).find(|dir| dir.join("tray.py").is_file()).map(Path::to_path_buf).unwrap_or(here)
}

const LOG_LIMIT: u64 = 512 * 1024;

/// Log to a file, rotated at 512 KB (one old copy kept).
struct FileLogger {
    path: PathBuf,
    file: Mutex<(File, u64)>,
    level: LevelFilter,
}

impl FileLogger {
    fn open(path: &Path) -> std::io::Result<(File, u64)> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let size = file.metadata().map_or(0, |m| m.len());
        Ok((file, size))
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            Level::Error => "ERROR",
            Level::Warn => "WARNING",
            Level::Info => "INFO",
            Level::Debug | Level::Trace => "DEBUG",
        };
        let line = format!("{} {} {}\n", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), level, record.args());
        let mut guard = self.file.lk();
        if guard.1 + line.len() as u64 > LOG_LIMIT {
            let old = self.path.with_extension("log.1");
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(&self.path, &old);
            if let Ok(fresh) = Self::open(&self.path) {
                *guard = fresh;
            }
        }
        if guard.0.write_all(line.as_bytes()).is_ok() {
            guard.1 += line.len() as u64;
        }
    }

    fn flush(&self) {
        let _ = self.file.lk().0.flush();
    }
}

/// Log to `path` (INFO and up; `GEEKMAGIC_LOG=debug` for more).
pub fn init_logging(path: &Path) {
    let level = match std::env::var("GEEKMAGIC_LOG").as_deref() {
        Ok("debug") => LevelFilter::Debug,
        Ok("warn") => LevelFilter::Warn,
        _ => LevelFilter::Info,
    };
    if let Ok(file) = FileLogger::open(path) {
        let logger = FileLogger { path: path.to_path_buf(), file: Mutex::new(file), level };
        if log::set_boxed_logger(Box::new(logger)).is_ok() {
            log::set_max_level(level);
        }
    }
}

pub fn build_registry() -> Arc<ProviderRegistry> {
    Arc::new(ProviderRegistry::new(default_providers()))
}

/// The real adapters behind every port, with `ui` as the tray.
pub fn real_ports(ui: Arc<dyn Ui>, state_path: &Path, scan: bool) -> Ports {
    Ports {
        providers: build_registry(),
        store: Arc::new(JsonStateStore::new(state_path)),
        display_factory: Arc::new(|ip| Arc::new(GeekMagicDevice::new(ip))),
        locator: if scan { Arc::new(Locator) } else { Arc::new(FixedLocator) },
        renderer: Arc::new(SoftRenderer::new()),
        notifier: Arc::new(SystemNotifier),
        lock_sensor: Arc::new(SessionLockSensor),
        terminal: Arc::new(Terminal),
        ui,
    }
}

/// How the tray app is asked to run.
pub struct TrayOptions {
    pub settings: Settings,
    pub data_dir: Option<PathBuf>,
    pub webhook_port: u16,
    pub webhook: bool,
    pub webhook_token: Option<String>,
    pub no_tray: bool,
    /// Look for the screen on the network when it isn't where it was (`--no-discovery` turns that off).
    pub discovery: bool,
}

/// Show the tray icon and keep the screen up to date, on webhooks, until it is quit. The process's exit code.
pub fn run_tray(options: TrayOptions) -> i32 {
    let paths = Paths { root: resolve_data_dir(options.data_dir.clone()) };
    // held for as long as the app runs: the lock goes with it
    let _instance = match single_instance::acquire(&paths.lock()) {
        Ok(lock) => lock,
        Err(_) => {
            eprintln!("The tray app is already running; not starting a second copy.");
            return 0;
        }
    };
    init_logging(&paths.log());
    log::info!("starting (data in {})", paths.root.display());

    #[cfg(windows)]
    let tray = (!options.no_tray).then(|| crate::adapters::tray::ui::TrayUi::new(paths.log()));
    #[cfg(windows)]
    let ui: Arc<dyn Ui> = match &tray {
        Some(t) => t.clone(),
        None => HeadlessUi::new(),
    };
    #[cfg(not(windows))]
    let ui: Arc<dyn Ui> = HeadlessUi::new();

    let controller = TrayController::new(real_ports(ui, &paths.state(), options.discovery), options.settings);
    #[cfg(windows)]
    if let Some(t) = &tray {
        t.attach(&controller);
    }
    if options.webhook {
        let handler = {
            let app = Arc::downgrade(&controller);
            Arc::new(move |event: WebhookEvent| {
                if let Some(app) = app.upgrade() {
                    app.on_webhook(event);
                }
            })
        };
        match webhook::start(options.webhook_port, options.webhook_token, controller.providers.keys(), handler) {
            Ok(server) => log::info!("webhook listening on http://{}", server.addr),
            Err(e) => log::warn!("the webhook could not start on port {}: {e} (clicks and the fallback refresh still work)", options.webhook_port),
        }
    }
    controller.run();
    log::info!("stopped");
    0
}
