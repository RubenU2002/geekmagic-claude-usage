//! The tray icon: the driving adapter that calls the application.

pub mod headless;
pub mod icons;
pub mod menu;

#[cfg(windows)]
pub mod dialog;
#[cfg(windows)]
pub mod simulator;
#[cfg(windows)]
pub mod ui;
