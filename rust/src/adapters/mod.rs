//! The adapters: each implements one of the core's ports (or, for the tray, drives the application).

pub mod device;
pub mod providers;
pub mod render;
pub mod store;
pub mod system;
pub mod tray;
pub mod webhook;
