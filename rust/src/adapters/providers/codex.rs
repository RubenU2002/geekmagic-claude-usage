//! Codex: its usage (from the account-limits call of its own app-server, which doesn't start a model turn).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, TimeZone, Utc};
use serde_json::{json, Value};

use super::codex_logs;
use super::executables::{home_dir, no_window, which};
use crate::core::errors::UsageError;
use crate::core::model::{CodexExtra, Usage, Window};
use crate::core::pace;
use crate::core::ports::{AgentState, Provider};
use crate::util::{epoch, pyround};

/// (like Claude's read: slow connections need more than 30 s)
pub const CODEX_TIMEOUT: Duration = Duration::from_secs(60);

/// `codex` from PATH, else the newest copy bundled with a VS Code-family ChatGPT/Codex extension.
pub fn find_codex() -> Option<String> {
    if let Some(found) = which("codex") {
        return Some(found);
    }
    let binary = if cfg!(windows) { "codex.exe" } else { "codex" };
    let mut bundled: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for editor in [".vscode", ".vscode-insiders", ".cursor"] {
        let Ok(extensions) = std::fs::read_dir(home_dir().join(editor).join("extensions")) else { continue };
        for extension in extensions.flatten() {
            if !extension.file_name().to_string_lossy().starts_with("openai.chatgpt-") {
                continue;
            }
            let Ok(platforms) = std::fs::read_dir(extension.path().join("bin")) else { continue };
            for platform in platforms.flatten() {
                let path = platform.path().join(binary);
                if let Ok(modified) = path.metadata().and_then(|m| m.modified()) {
                    bundled.push((path, modified));
                }
            }
        }
    }
    bundled.sort_by(|a, b| b.1.cmp(&a.1));
    bundled.into_iter().next().map(|(p, _)| p.to_string_lossy().into_owned())
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

/// Query the authenticated Codex app-server without starting an AI turn. Returns its limits and the free-reset credits.
fn fetch_rate_limits() -> Result<(Value, Value), UsageError> {
    let executable = find_codex().ok_or_else(|| UsageError::new("`codex` CLI not found in PATH. Install Codex CLI and run `codex login`."))?;
    let mut cmd = Command::new(executable);
    cmd.arg("app-server").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = no_window(&mut cmd).spawn().map_err(|_| UsageError::new("Could not start `codex app-server`."))?;
    let mut stdin = child.stdin.take().ok_or_else(|| UsageError::new("Could not start `codex app-server`."))?;
    let stdout = child.stdout.take().ok_or_else(|| UsageError::new("Could not start `codex app-server`."))?;

    let (tx, responses) = mpsc::channel::<Option<String>>();
    let reader = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if tx.send(Some(line)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(None);
    });
    let deadline = Instant::now() + CODEX_TIMEOUT;

    let timed_out = || UsageError::new("Timed out querying Codex limits. Check your connection and `codex login`.");
    let mut send = |message: Value| -> Result<(), UsageError> {
        writeln!(stdin, "{message}").and_then(|_| stdin.flush()).map_err(|_| UsageError::new("Lost connection to Codex app-server."))
    };
    let receive = |id: i64| -> Result<Value, UsageError> {
        loop {
            let remaining = deadline.checked_duration_since(Instant::now()).ok_or_else(timed_out)?;
            let line = match responses.recv_timeout(remaining) {
                Ok(Some(line)) => line,
                Ok(None) => return Err(UsageError::new("Codex app-server exited before returning limits. Check `codex login`.")),
                Err(_) => return Err(timed_out()),
            };
            let message: Value = serde_json::from_str(&line).map_err(|_| UsageError::new("Codex app-server returned invalid JSON."))?;
            if message.get("id").and_then(Value::as_i64) != Some(id) {
                continue; // Notifications can arrive between request responses.
            }
            if message.get("error").is_some() {
                return Err(UsageError::sign_in("Codex could not read account limits. Not signed in? Run `codex login` with ChatGPT."));
            }
            return match message.get("result") {
                Some(r) if r.is_object() => Ok(r.clone()),
                _ => Err(UsageError::new("Codex app-server returned an invalid response.")),
            };
        }
    };

    let outcome = (|| {
        send(json!({"id": 1, "method": "initialize", "params": {"clientInfo": {
            "name": "geekmagic_usage", "title": "GeekMagic Usage", "version": "1.0.0"}}}))?;
        receive(1)?;
        send(json!({"method": "initialized", "params": {}}))?;
        send(json!({"id": 2, "method": "account/rateLimits/read"}))?;
        let result = receive(2)?;
        // Prefer the Codex bucket when multiple model quotas are returned.
        let mut limits = result.get("rateLimitsByLimitId").and_then(|b| b.get("codex")).cloned().filter(|l| !l.is_null());
        if limits.is_none() {
            limits = result.get("rateLimits").cloned();
        }
        match limits {
            Some(l) if l.is_object() && ["primary", "secondary"].iter().any(|w| truthy(l.get(*w))) => {
                Ok((l, result.get("rateLimitResetCredits").cloned().unwrap_or(Value::Null))) // the free resets sit beside the limits
            }
            _ => Err(UsageError::sign_in("Codex returned no account limits. Sign in with ChatGPT using `codex login`.")),
        }
    })();

    drop(stdin);
    let waited = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < waited => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    if let Ok(handle) = reader {
        let _ = handle.join();
    }
    outcome
}

fn window(block: Option<&Value>) -> Result<Window, UsageError> {
    let invalid = || UsageError::new("Codex returned an invalid quota window.");
    let block = match block {
        None | Some(Value::Null) => return Ok(Window::default()),
        Some(b) if b.is_object() => b,
        Some(_) => return Err(invalid()),
    };
    let reset: Option<DateTime<Utc>> = match block.get("resetsAt") {
        Some(v) if truthy(Some(v)) => {
            let secs = v.as_f64().ok_or_else(invalid)?;
            Some(Local.timestamp_opt(secs as i64, 0).single().ok_or_else(invalid)?.with_timezone(&Utc))
        }
        _ => None,
    };
    let pct = block.get("usedPercent").and_then(Value::as_f64).ok_or_else(invalid)?;
    // Codex says how long each window really is (300 and 10080 minutes at the time of writing).
    let minutes = block.get("windowDurationMins").and_then(Value::as_f64);
    Ok(Window { pct: Some(pct), reset, minutes })
}

/// The free rate-limit resets Codex has granted you, for its own screen: how many are left and in how many days the next
/// one expires. Every field is optional in Codex's reply.
fn codex_extra(credits: &Value) -> CodexExtra {
    let empty = Vec::new();
    let list = credits.get("credits").and_then(Value::as_array).unwrap_or(&empty); // (sometimes null instead of left out)
    let available: Vec<&Value> = list.iter().filter(|c| c.is_object() && c.get("status").and_then(Value::as_str) == Some("available")).collect();
    let expiries: Vec<f64> = available.iter().filter_map(|c| c.get("expiresAt").and_then(Value::as_f64)).collect();
    let free_resets = match credits.get("availableCount") {
        Some(v) => v.as_f64().map_or(0, |f| f as i64),
        None => available.len() as i64,
    };
    let soonest = expiries.iter().copied().fold(f64::INFINITY, f64::min);
    CodexExtra {
        free_resets,
        next_reset_expires_days: soonest.is_finite().then(|| pyround((soonest - epoch()) / 86_400.0).max(0)),
    }
}

/// Fetch fresh account limits, even when the user is not actively using Codex.
pub fn fetch_codex_usage() -> Result<Usage, UsageError> {
    let (limits, credits) = fetch_rate_limits()?;
    let mut usage = Usage::new("Codex", window(limits.get("primary"))?, window(limits.get("secondary"))?, Local::now());
    usage.codex_extra = Some(codex_extra(&credits));
    pace::annotate(&mut usage, [None, None]);
    Ok(usage)
}

pub struct CodexProvider;

impl Provider for CodexProvider {
    fn key(&self) -> &str {
        "codex"
    }
    fn title(&self) -> &str {
        "Codex"
    }
    fn login_args(&self) -> &[&str] {
        &["login"]
    }
    fn install_hint(&self) -> &str {
        "Install the Codex CLI, then run `codex login`."
    }
    fn fetch(&self) -> Result<Usage, UsageError> {
        fetch_codex_usage()
    }
    fn find_cli(&self) -> Option<String> {
        find_codex()
    }
    fn local_stats(&self, now_epoch: f64) -> Option<crate::core::activity::Summary> {
        codex_logs::stats(now_epoch, None)
    }
    fn agent_state(&self, now_epoch: f64) -> AgentState {
        codex_logs::agent_state(now_epoch, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_parse_and_missing_ones_are_empty() {
        let w = window(Some(&json!({"usedPercent": 12.5, "resetsAt": 1_790_953_020, "windowDurationMins": 300}))).unwrap();
        assert_eq!((w.pct, w.minutes, w.reset.unwrap().timestamp()), (Some(12.5), Some(300.0), 1_790_953_020));
        assert!(window(None).unwrap().pct.is_none());
        assert!(window(Some(&Value::Null)).unwrap().pct.is_none());
        let no_reset = window(Some(&json!({"usedPercent": 3}))).unwrap();
        assert_eq!((no_reset.pct, no_reset.reset.is_none(), no_reset.minutes), (Some(3.0), true, None));
        assert!(window(Some(&json!("nope"))).is_err());
        assert!(window(Some(&json!({"resetsAt": 5}))).is_err(), "usedPercent is required");
    }

    #[test]
    fn free_resets_count_and_next_expiry() {
        let in_days = |d: f64| epoch() + d * 86_400.0;
        let credits = json!({"availableCount": 2, "credits": [
            {"status": "available", "expiresAt": in_days(2.2)},
            {"status": "available", "expiresAt": in_days(9.0)},
            {"status": "used", "expiresAt": in_days(0.1)},
        ]});
        let e = codex_extra(&credits);
        assert_eq!((e.free_resets, e.next_reset_expires_days), (2, Some(2)));
        let none = codex_extra(&json!({"credits": null}));
        assert_eq!((none.free_resets, none.next_reset_expires_days), (0, None));
        assert_eq!(codex_extra(&Value::Null).free_resets, 0);
        assert_eq!(codex_extra(&json!({"credits": [{"status": "available"}]})).free_resets, 1, "counted when the count is missing");
        assert_eq!(codex_extra(&json!({"credits": [{"status": "available", "expiresAt": in_days(-3.0)}]})).next_reset_expires_days, Some(0));
    }

    #[test]
    fn truthiness_follows_python() {
        assert!(!truthy(None) && !truthy(Some(&json!(null))) && !truthy(Some(&json!({}))) && !truthy(Some(&json!(0))));
        assert!(truthy(Some(&json!({"usedPercent": 0}))) && truthy(Some(&json!(5))));
    }
}
