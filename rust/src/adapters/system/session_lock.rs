//! Is this computer's screen locked? Best effort per OS; anything unknown or failing counts as "not locked".

use crate::core::ports::LockSensor;

/// While the workstation is locked the input desktop is the secure one, which a normal process may neither open nor
/// switch to.
#[cfg(windows)]
fn locked() -> bool {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::StationsAndDesktops::{CloseDesktop, OpenInputDesktop, SwitchDesktop, DESKTOP_SWITCHDESKTOP};
    const ERROR_ACCESS_DENIED: u32 = 5;
    // SAFETY: plain Win32 calls; the desktop handle is closed before returning.
    unsafe {
        let desktop = OpenInputDesktop(0, 0, DESKTOP_SWITCHDESKTOP);
        if desktop as usize == 0 {
            return GetLastError() == ERROR_ACCESS_DENIED; // any other failure: don't claim it's locked
        }
        let switched = SwitchDesktop(desktop);
        CloseDesktop(desktop);
        switched == 0
    }
}

/// systemd-logind's LockedHint (set by GNOME, KDE and most lockers).
#[cfg(target_os = "linux")]
fn locked() -> bool {
    let Ok(session) = std::env::var("XDG_SESSION_ID") else { return false };
    std::process::Command::new("loginctl")
        .args(["show-session", &session, "-p", "LockedHint", "--value"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "yes")
        .unwrap_or(false)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn locked() -> bool {
    false
}

pub fn is_locked() -> bool {
    locked()
}

/// The LockSensor port: is this computer's screen locked right now.
pub struct SessionLockSensor;

impl LockSensor for SessionLockSensor {
    fn is_locked(&self) -> bool {
        is_locked()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_unlocked_session_running_the_tests_is_not_locked() {
        // (the tests run in a logged-in session: asking must work and say "no")
        assert!(!super::is_locked());
    }
}
