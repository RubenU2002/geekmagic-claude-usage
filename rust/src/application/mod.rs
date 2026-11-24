//! The application: the use cases. It is written against the core's ports and knows no adapter.

use std::sync::Arc;

pub mod activity;
pub mod agents;
pub mod backlight;
pub mod capture;
pub mod config;
pub mod controller;
pub mod image_files;
pub mod local_screen;
pub mod notifications;
pub mod persistence;
pub mod power;
pub mod scheduler;
pub mod screen;
pub mod signin;
pub mod usage;
pub mod viewstate;

#[cfg(test)]
pub mod testing;

/// Tell the user something: (title, message).
pub type Notify = Arc<dyn Fn(&str, &str) + Send + Sync>;
/// Something to call when something happened.
pub type Callback = Arc<dyn Fn() + Send + Sync>;
