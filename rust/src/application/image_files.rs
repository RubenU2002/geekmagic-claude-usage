//! The names of the image files kept on the device.

/// From before the GIF switch; cleaned up once.
pub const LEGACY_NAMES: [&str; 1] = ["claude-usage.jpg"];

/// The one file of a provider the command-line mode writes (the tray app alternates between two, see `slot_file`).
pub fn image_name(key: &str) -> String {
    format!("{key}-usage.gif")
}

/// One of the two files a view alternates between (slot "a" / "b"): an upload cut short by a click can only damage the
/// spare one, so the one on screen (and the one a switch will show) stays intact.
pub fn slot_file(key: &str, slot: &str) -> String {
    format!("{key}-usage-{slot}.gif")
}
