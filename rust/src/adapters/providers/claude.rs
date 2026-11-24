//! Claude Code: its usage (from its own built-in /usage, which costs nothing).

use std::process::Command;
use std::str::FromStr;
use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;
use serde_json::Value;

use super::claude_logs;
use super::executables::{run_with_timeout, which, RunError};
use crate::core::errors::UsageError;
use crate::core::model::{Usage, Window};
use crate::core::pace;
use crate::core::ports::{AgentState, Provider};

static SESSION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^Current session:\s*(\d+(?:\.\d+)?)% used(?:\s*·\s*resets\s+(.+))?$").unwrap());
static WEEK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^Current week(?:\s*\([^)]*\))?:\s*(\d+(?:\.\d+)?)% used(?:\s*·\s*resets\s+(.+))?$").unwrap());
static RESET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?P<month>[A-Za-z]{3})\s+(?P<day>\d{1,2})(?:,|\s+at)\s+(?P<hour>\d{1,2})(?::(?P<minute>\d{2}))?(?P<meridiem>am|pm)\s+\((?P<tz>[^)]+)\)",
    )
    .unwrap()
});
/// What Claude Code says when it isn't signed in ("Not logged in · Please run /login"), however it phrases it.
static SIGNED_OUT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)not logged in|/login|log ?in again|sign ?in|authenticat|invalid api key|unauthori[sz]ed|token .*expired").unwrap()
});

/// It takes ~10 s, but 30 s or more on a slow connection: a timeout costs a whole cycle for nothing.
pub const TIMEOUT: Duration = Duration::from_secs(90);

/// Call Claude Code's built-in /usage command. Guaranteed $0 cost.
pub fn fetch_usage() -> Result<Usage, UsageError> {
    let mut cmd = Command::new(which("claude").unwrap_or_else(|| "claude".into()));
    cmd.args([
        "-p", "--safe-mode", "--output-format", "json", "--max-budget-usd", "0.000001", "--tools", "", "--no-session-persistence",
        "--no-chrome", "/usage",
    ]);
    let out = run_with_timeout(&mut cmd, TIMEOUT).map_err(|e| match e {
        RunError::NotFound => UsageError::new("`claude` CLI not found in PATH. Install/open Claude Code first."),
        RunError::TimedOut => UsageError::new("Timed out waiting for `claude /usage`."),
        RunError::Other(m) => UsageError::new(format!("Could not run `claude`: {m}")),
    })?;
    if out.code != Some(0) {
        if SIGNED_OUT_RE.is_match(&format!("{} {}", out.stdout, out.stderr)) {
            return Err(UsageError::sign_in("Claude Code isn't signed in."));
        }
        let code = out.code.map_or("without a code".to_string(), |c| c.to_string());
        return Err(UsageError::new(format!("claude exited {code}: {}", out.stderr.trim())));
    }
    let payload: Value =
        serde_json::from_str(&out.stdout).map_err(|_| UsageError::new("`claude /usage` did not return JSON."))?;
    let cost = payload.get("total_cost_usd").map_or(0.0, |c| c.as_f64().or_else(|| c.as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0));
    if cost != 0.0 {
        return Err(UsageError::new("Refusing result: /usage unexpectedly incurred cost."));
    }
    let is_error = payload.get("is_error").is_some_and(|v| v.as_bool().unwrap_or(!v.is_null()));
    let Some(result) = payload.get("result").and_then(Value::as_str).filter(|_| !is_error) else {
        let shown = payload.get("result").map_or("None".to_string(), |r| r.as_str().map_or(r.to_string(), String::from));
        if SIGNED_OUT_RE.is_match(&shown) {
            return Err(UsageError::sign_in("Claude Code isn't signed in."));
        }
        return Err(UsageError::new("Claude Code did not return usage text."));
    };
    let mut usage = parse(result)?;
    pace::annotate(&mut usage, [None, None]);
    Ok(usage)
}

fn window_of(m: Option<regex::Captures>, now: DateTime<Local>) -> Window {
    let Some(m) = m else { return Window::default() };
    Window {
        pct: m[1].parse().ok(),
        reset: m.get(2).and_then(|r| parse_reset(r.as_str(), now)),
        minutes: None,
    }
}

pub fn parse(text: &str) -> Result<Usage, UsageError> {
    let now = Local::now();
    let (session, week) = (SESSION_RE.captures(text), WEEK_RE.captures(text));
    if session.is_none() && week.is_none() {
        return Err(UsageError::new(format!("Could not find usage lines in /usage output:\n{text}")));
    }
    Ok(Usage::new("Claude", window_of(session, now), window_of(week, now), now))
}

fn month_number(abbreviation: &str) -> Option<u32> {
    let names = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    names.iter().position(|n| *n == abbreviation.to_lowercase()).map(|i| i as u32 + 1)
}

/// "Oct 2, 3pm (America/Bogota)" -> that moment: the next one on or after `now` (this year, else next).
pub fn parse_reset(text: &str, now: DateTime<Local>) -> Option<DateTime<Utc>> {
    let m = RESET_RE.captures(text)?;
    let tz = Tz::from_str(m.name("tz")?.as_str()).ok()?;
    let month = month_number(m.name("month")?.as_str())?;
    let day: u32 = m.name("day")?.as_str().parse().ok()?;
    let mut hour: u32 = m.name("hour")?.as_str().parse::<u32>().ok()? % 12;
    if m.name("meridiem")?.as_str().eq_ignore_ascii_case("pm") {
        hour += 12;
    }
    let minute: u32 = m.name("minute").map_or(Some(0), |x| x.as_str().parse().ok())?;
    let local_now = now.with_timezone(&tz);
    let mut candidate = None;
    for year in [local_now.year(), local_now.year() + 1] {
        let Some(moment) = tz.with_ymd_and_hms(year, month, day, hour, minute, 0).earliest() else { continue };
        let moment = moment.with_timezone(&Utc);
        candidate = Some(moment);
        if moment >= now.with_timezone(&Utc) {
            return Some(moment);
        }
    }
    candidate
}

pub struct ClaudeProvider;

impl Provider for ClaudeProvider {
    fn key(&self) -> &str {
        "claude"
    }
    fn title(&self) -> &str {
        "Claude"
    }
    fn login_args(&self) -> &[&str] {
        &["auth", "login"]
    }
    fn install_hint(&self) -> &str {
        "Install Claude Code (https://claude.ai/code), then sign in."
    }
    fn fetch(&self) -> Result<Usage, UsageError> {
        fetch_usage()
    }
    fn find_cli(&self) -> Option<String> {
        which("claude")
    }
    fn local_stats(&self, now_epoch: f64) -> Option<crate::core::activity::Summary> {
        claude_logs::stats(now_epoch, None)
    }
    fn agent_state(&self, now_epoch: f64) -> AgentState {
        claude_logs::agent_state(now_epoch, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "Current session: 42% used · resets Oct 2, 3pm (America/Bogota)\nCurrent week (all models): 17.5% used · resets Oct 9, 10:30am (America/Bogota)\n";

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn parses_both_windows() {
        let u = parse(OUTPUT).unwrap();
        assert_eq!((u.current.pct, u.weekly.pct), (Some(42.0), Some(17.5)));
        let bogota: Tz = "America/Bogota".parse().unwrap();
        let session = u.current.reset.unwrap().with_timezone(&bogota);
        assert_eq!((session.month(), session.day(), chrono::Timelike::hour(&session)), (10, 2, 15));
        let week = u.weekly.reset.unwrap().with_timezone(&bogota);
        assert_eq!((week.day(), chrono::Timelike::hour(&week), chrono::Timelike::minute(&week)), (9, 10, 30));
    }

    #[test]
    fn a_missing_window_or_reset_is_none_and_no_lines_is_an_error() {
        let u = parse("Current session: 5% used\n").unwrap();
        assert_eq!((u.current.pct, u.current.reset.is_none(), u.weekly.pct), (Some(5.0), true, None));
        assert!(parse("nothing useful").unwrap_err().message.contains("Could not find usage lines"));
    }

    #[test]
    fn resets_roll_into_next_year_and_12am_12pm_are_right() {
        let now = at(2026, 12, 30, 12, 0);
        let utc = |m: &str| parse_reset(m, now).unwrap().with_timezone(&chrono_tz::UTC);
        let jan = utc("Jan 2, 12am (UTC)");
        assert_eq!((jan.year(), jan.month(), chrono::Timelike::hour(&jan)), (2027, 1, 0));
        assert_eq!(chrono::Timelike::hour(&utc("Dec 31, 12pm (UTC)")), 12);
        assert_eq!(parse_reset("garbage", now), None);
        assert_eq!(parse_reset("Oct 2, 3pm (Not/AZone)", now), None);
    }

    #[test]
    fn signed_out_phrasings() {
        for text in ["Not logged in · Please run /login", "please sign in", "Invalid API key", "401 Unauthorized", "token has expired"] {
            assert!(SIGNED_OUT_RE.is_match(text), "{text}");
        }
        assert!(!SIGNED_OUT_RE.is_match("Current session: 4% used"));
    }
}
