//! GeekMagic usage screen for Claude Code and Codex: the Rust port (hexagonal, like the Python original), refreshed by
//! webhooks instead of on a timer.

// (style lints about long signatures and nested trait-object types: the ports are meant to read like that)
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

pub mod adapters;
pub mod application;
pub mod bootstrap;
pub mod cli;
pub mod core;
pub mod util;
