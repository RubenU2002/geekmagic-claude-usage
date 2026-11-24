//! Finding a command-line tool, including where a login task's short PATH wouldn't look; and running one with a timeout.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(windows)]
fn extensions() -> Vec<String> {
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    pathext.split(';').filter(|e| !e.is_empty()).map(|e| e.to_lowercase()).collect()
}

#[cfg(not(windows))]
fn extensions() -> Vec<String> {
    Vec::new()
}

fn find_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.is_file() && (!cfg!(windows) || Path::new(name).extension().is_some()) {
        return Some(exact);
    }
    extensions().into_iter().map(|ext| dir.join(format!("{name}{ext}"))).find(|p| p.is_file())
}

/// Like `which`, plus the usual install folders a background job (Task Scheduler, launchd) may not have on PATH.
pub fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let extra = [home_dir().join(".local").join("bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")];
    std::env::split_paths(&path)
        .chain(extra)
        .find_map(|dir| find_in(&dir, name))
        .map(|p| p.to_string_lossy().into_owned())
}

/// Keep a child process from opening a console window (Windows).
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

pub struct Finished {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub enum RunError {
    NotFound,
    TimedOut,
    Other(String),
}

fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}

/// Run `cmd` to completion, killing it after `timeout`. Its output is read as it runs, so a chatty child can't block.
pub fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Result<Finished, RunError> {
    no_window(cmd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => RunError::NotFound,
        _ => RunError::Other(e.to_string()),
    })?;
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::TimedOut);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(RunError::Other(e.to_string())),
        }
    };
    // (a grandchild may still hold the pipes open: don't wait for them for long)
    let take = |rx: Option<mpsc::Receiver<Vec<u8>>>| {
        rx.and_then(|rx| rx.recv_timeout(Duration::from_secs(2)).ok()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
    };
    Ok(Finished { code: status.code(), stdout: take(out), stderr: take(err) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_finds_a_system_tool_and_not_a_missing_one() {
        let tool = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(which(tool).is_some());
        assert!(which("definitely-not-installed-xyz").is_none());
    }

    #[test]
    fn runs_with_output_and_times_out() {
        let mut ok = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/c", "echo hello"]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", "echo hello"]);
            c
        };
        let done = run_with_timeout(&mut ok, Duration::from_secs(10)).unwrap();
        assert_eq!((done.code, done.stdout.trim()), (Some(0), "hello"));
        let mut slow = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "6", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("5");
            c
        };
        assert!(matches!(run_with_timeout(&mut slow, Duration::from_millis(300)), Err(RunError::TimedOut)));
        assert!(matches!(run_with_timeout(&mut Command::new("definitely-not-installed-xyz"), Duration::from_secs(1)), Err(RunError::NotFound)));
    }
}
