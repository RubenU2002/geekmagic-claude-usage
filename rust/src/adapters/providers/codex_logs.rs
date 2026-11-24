//! Codex's local logs (`~/.codex/sessions/**/*.jsonl`): what you did with it, and whether it is working right now.
//!
//! Codex marks every turn with `task_started` and `task_complete` (or `turn_aborted`); an approval or question request
//! that is the last thing in the log means it is waiting (best effort: not every setup records those).

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::executables::home_dir;
use super::logs::{collect, epoch, scan_state, tail_lines, EventCache, SummaryCache, TOOL_MAX_AGE, TURN_MAX_AGE, WAIT_MAX_AGE};
use crate::core::activity::{intern, project_label, summarize, Ev, Summary};
use crate::core::ports::AgentState;
use crate::core::sessions::SessionState;

pub fn codex_dir() -> PathBuf {
    home_dir().join(".codex").join("sessions")
}

static TIMESTAMP_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""timestamp"\s*:\s*"([^"]+)""#).unwrap());
static PAYLOAD_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""payload"\s*:\s*\{\s*"type"\s*:\s*"([A-Za-z_]+)""#).unwrap());

fn line_timestamp(line: &str) -> Option<f64> {
    TIMESTAMP_RE.captures(line).and_then(|m| epoch(&m[1]))
}

/// Every model call in one Codex session log: a `token_count` event that carries token info. Which project and model it
/// belongs to is whatever the latest `turn_context` / `session_meta` before it said. Streamed line by line.
pub fn read_events(path: &Path) -> io::Result<Vec<Ev>> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let session = intern(&stem);
    let mut reader = BufReader::new(File::open(path)?);
    let mut found = Vec::new();
    let (mut project, mut model) = (intern("(unknown)"), intern("(unknown)"));
    let mut raw = Vec::new();
    loop {
        raw.clear();
        if reader.read_until(b'\n', &mut raw)? == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&raw);
        if line.contains("\"turn_context\"") || line.contains("\"session_meta\"") {
            let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(payload) = entry.get("payload").filter(|p| p.is_object()) {
                if let Some(cwd) = payload.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
                    project = intern(&project_label(Some(cwd)));
                }
                if let Some(m) = payload.get("model").and_then(Value::as_str).filter(|m| !m.is_empty()) {
                    model = intern(m);
                }
            }
        } else if line.contains("\"token_count\"") && !line.replace(' ', "").contains("\"info\":null") {
            if let Some(when) = line_timestamp(&line) {
                found.push(Ev { when, session: session.clone(), id: None, project: project.clone(), model: model.clone() });
            }
        }
    }
    Ok(found)
}

static EVENTS: LazyLock<EventCache> = LazyLock::new(EventCache::default);

pub fn stats(now_epoch: f64, root: Option<&Path>) -> Option<Summary> {
    let root = root.map(Path::to_path_buf).unwrap_or_else(codex_dir);
    root.is_dir().then(|| summarize(&collect(&root, read_events, &EVENTS, now_epoch), now_epoch))
}

/// Codex events that wait on you.
const CODEX_REQUESTS: [&str; 3] = ["approval_request", "request_user_input", "elicitation_request"];

#[derive(Debug)]
pub struct TailSummary {
    pub marker: Option<&'static str>,
    pub marker_time: Option<f64>,
    pub last: f64,
    pub last_type: Option<String>,
    pub project: Option<String>,
    pub session: String,
}

pub fn summary(path: &Path) -> io::Result<Option<TailSummary>> {
    let (mut marker, mut marker_time, mut last_time, mut last_type, mut project) = (None, None, None, None, None);
    for line in tail_lines(path)? {
        let found = ["task_started", "task_complete", "turn_aborted"].into_iter().find(|k| line.contains(&format!("\"{k}\"")));
        if line.contains("\"turn_context\"") {
            if let Ok(entry) = serde_json::from_str::<Value>(&line) {
                let cwd = entry.get("payload").and_then(|p| p.get("cwd")).and_then(Value::as_str).filter(|c| !c.is_empty());
                if let Some(cwd) = cwd {
                    project = Some(project_label(Some(cwd)));
                }
            }
        }
        let when = line_timestamp(&line);
        if when.is_some() {
            last_time = when;
        }
        if let Some(m) = PAYLOAD_TYPE_RE.captures(&line) {
            last_type = Some(m[1].to_string());
        }
        if found.is_some() {
            marker = found;
            marker_time = when;
        }
    }
    let Some(last) = last_time else { return Ok(None) };
    let session = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Some(TailSummary { marker, marker_time, last, last_type, project, session }))
}

pub fn decide(summary: &TailSummary, now_epoch: f64) -> Option<SessionState> {
    let age = now_epoch - summary.last;
    let Some(marker) = summary.marker else {
        // a long run whose markers have scrolled out of the tail: judge by recent writes
        return Some(SessionState {
            id: summary.session.clone(),
            working: age <= TURN_MAX_AGE,
            last: summary.last,
            project: summary.project.clone(),
            ..Default::default()
        });
    };
    let in_turn = marker == "task_started";
    let last_type = summary.last_type.as_deref().unwrap_or("");
    let asking = in_turn && age <= WAIT_MAX_AGE && CODEX_REQUESTS.iter().any(|k| last_type.contains(k));
    let working = in_turn && (asking || age <= TOOL_MAX_AGE);
    Some(SessionState {
        id: summary.session.clone(),
        working,
        waiting: asking,
        waiting_for: asking.then(|| "approval".to_string()),
        since: if working { summary.marker_time } else { None },
        last: summary.last,
        project: summary.project.clone(),
    })
}

static SUMMARIES: LazyLock<SummaryCache<TailSummary>> = LazyLock::new(SummaryCache::default);

pub fn agent_state(now_epoch: f64, root: Option<&Path>) -> AgentState {
    let root = root.map(Path::to_path_buf).unwrap_or_else(codex_dir);
    scan_state(&root, summary, decide, &SUMMARIES, now_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gm-codex-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ev(ts: &str, kind: &str, extra: &str) -> String {
        format!(r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"{kind}"{extra}}}}}"#)
    }

    #[test]
    fn events_follow_the_latest_context_and_skip_empty_token_counts() {
        let dir = temp("events");
        let log = [
            r#"{"timestamp":"2026-10-02T09:00:00Z","type":"session_meta","payload":{"cwd":"C:\\Work\\Beta"}}"#.to_string(),
            r#"{"timestamp":"2026-10-02T09:00:01Z","type":"turn_context","payload":{"cwd":"C:\\Work\\Beta","model":"gpt-5-codex"}}"#.to_string(),
            ev("2026-10-02T09:01:00Z", "token_count", r#","info":{"total":1}"#),
            ev("2026-10-02T09:02:00Z", "token_count", r#","info":null"#),
            ev("2026-10-02T09:03:00Z", "token_count", r#","info": null"#),
            ev("2026-10-02T09:04:00Z", "token_count", r#","info":{"total":2}"#),
        ]
        .join("\n");
        let path = dir.join("rollout-1.jsonl");
        fs::write(&path, log).unwrap();
        let events = read_events(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!((&*events[0].project, &*events[0].model, &*events[0].session), ("Beta", "gpt-5-codex", "rollout-1"));
        assert!(events.iter().all(|e| e.id.is_none()));
        fs::remove_dir_all(&dir).unwrap();
    }

    fn write(dir: &Path, lines: &[String]) -> PathBuf {
        let p = dir.join("rollout-2.jsonl");
        fs::write(&p, lines.join("\n")).unwrap();
        p
    }

    #[test]
    fn a_turn_in_progress_works_and_a_finished_one_does_not() {
        let dir = temp("turn");
        let at = |s: &str| epoch(s).unwrap();
        let p = write(&dir, &[ev("2026-10-02T09:00:00Z", "task_started", ""), ev("2026-10-02T09:00:30Z", "agent_message", "")]);
        let s = decide(&summary(&p).unwrap().unwrap(), at("2026-10-02T09:05:00Z")).unwrap();
        assert!(s.working && !s.waiting);
        assert_eq!(s.since, Some(at("2026-10-02T09:00:00Z")));
        assert!(!decide(&summary(&p).unwrap().unwrap(), at("2026-10-02T09:12:00Z")).unwrap().working, "silent past a tool's allowance");
        let p = write(&dir, &[ev("2026-10-02T09:00:00Z", "task_started", ""), ev("2026-10-02T09:00:30Z", "task_complete", "")]);
        assert!(!decide(&summary(&p).unwrap().unwrap(), at("2026-10-02T09:00:40Z")).unwrap().working);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_approval_request_at_the_end_means_waiting() {
        let dir = temp("wait");
        let p = write(&dir, &[ev("2026-10-02T09:00:00Z", "task_started", ""), ev("2026-10-02T09:00:30Z", "exec_approval_request", "")]);
        let s = decide(&summary(&p).unwrap().unwrap(), epoch("2026-10-02T09:30:00Z").unwrap()).unwrap();
        assert_eq!((s.working, s.waiting, s.waiting_for.as_deref()), (true, true, Some("approval")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_markers_recent_writes_decide() {
        let dir = temp("nomarker");
        let p = write(&dir, &[ev("2026-10-02T09:00:00Z", "agent_message", "")]);
        let sum = summary(&p).unwrap().unwrap();
        assert!(decide(&sum, epoch("2026-10-02T09:01:00Z").unwrap()).unwrap().working);
        assert!(!decide(&sum, epoch("2026-10-02T09:10:00Z").unwrap()).unwrap().working);
        assert!(summary(&write(&dir, &["no timestamps here".into()])).unwrap().is_none());
        fs::remove_dir_all(&dir).unwrap();
    }
}
