//! What can go wrong reading a provider's usage, or talking to the screen.

use std::fmt;

#[derive(Debug, Clone)]
pub struct UsageError {
    pub message: String,
    /// You're signed out (or the session expired): signing in again fixes it.
    pub sign_in_needed: bool,
}

impl UsageError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), sign_in_needed: false }
    }
    pub fn sign_in(message: impl Into<String>) -> Self {
        Self { message: message.into(), sign_in_needed: true }
    }
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for UsageError {}

/// How a call to the screen failed. `Offline` is the "device didn't answer properly" family (Python's OSError).
#[derive(Debug, Clone)]
pub enum DeviceError {
    /// An upload aborted because something more urgent (a click) needed the device.
    Cancelled,
    Offline(String),
    Other(String),
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::Cancelled => f.write_str("upload cancelled"),
            DeviceError::Offline(m) | DeviceError::Other(m) => f.write_str(m),
        }
    }
}
impl std::error::Error for DeviceError {}

impl From<std::io::Error> for DeviceError {
    fn from(e: std::io::Error) -> Self {
        DeviceError::Offline(e.to_string())
    }
}
