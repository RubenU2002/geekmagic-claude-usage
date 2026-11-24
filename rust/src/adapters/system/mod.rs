//! Everything that is the operating system's: notifications, the lock screen, one-instance, terminals, start at login.

pub mod autostart;
pub mod notifier;
pub mod session_lock;
pub mod single_instance;
pub mod terminal;
