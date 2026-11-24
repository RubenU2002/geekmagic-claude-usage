//! Only one tray app at a time.
//!
//! Two copies would fight over the screen (each uploading its own images), read the providers twice and send every
//! notification twice. The first one to start holds a lock on a file for as long as it lives; a second finds it held and
//! exits. The operating system drops the lock when the process ends, however it ends, so there's nothing to clean up
//! after a crash. It is the same file, and the same kind of byte-range lock, the Python version uses: the two apps can't
//! run at once either.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug)]
pub struct AlreadyRunning(pub String);

/// The held lock: keep it for as long as the app runs.
pub struct Lock {
    file: File,
}

#[cfg(windows)]
fn lock_byte(file: &File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY};
    use windows_sys::Win32::System::IO::OVERLAPPED;
    // SAFETY: the handle is valid for the call, and the zeroed OVERLAPPED asks for offset 0.
    unsafe {
        let mut overlapped: OVERLAPPED = std::mem::zeroed();
        let ok = LockFileEx(file.as_raw_handle() as _, LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY, 0, 1, 0, &mut overlapped);
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn lock_byte(_file: &File) -> std::io::Result<()> {
    Ok(()) // (only the Windows tray exists so far)
}

/// Take the lock on `path`. Fails with `AlreadyRunning` if another process holds it.
pub fn acquire(path: &Path) -> Result<Lock, AlreadyRunning> {
    let mut file = OpenOptions::new().read(true).append(true).create(true).open(path).map_err(|e| AlreadyRunning(e.to_string()))?;
    let mut first = [0u8; 1];
    if file.read(&mut first).unwrap_or(0) == 0 {
        let _ = file.write_all(&[0]); // a byte to lock (Windows locks byte ranges)
    }
    lock_byte(&file).map_err(|_| AlreadyRunning(format!("another instance holds {}", path.display())))?;
    Ok(Lock { file })
}

impl Lock {
    /// Let go early (the tests do; the app just exits).
    pub fn release(self) {
        drop(self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_is_refused_until_the_first_lets_go() {
        let path = std::env::temp_dir().join(format!("gm-lock-{}.lock", std::process::id()));
        let first = acquire(&path).unwrap();
        assert!(acquire(&path).is_err(), "held");
        first.release();
        let again = acquire(&path).expect("free again");
        again.release();
        let _ = std::fs::remove_file(&path);
    }
}
