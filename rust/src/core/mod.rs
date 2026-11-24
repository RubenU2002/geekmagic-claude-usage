//! The core: what the app is about. The model, the rules (pure functions) and the ports (traits) it needs from outside.
//! Knows nothing of any adapter.

pub mod activity;
pub mod alerts;
pub mod errors;
pub mod model;
pub mod pace;
pub mod ports;
pub mod registry;
pub mod sessions;
pub mod themes;
pub mod views;
