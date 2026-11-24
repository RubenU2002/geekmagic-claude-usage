//! Claude Code's local logs (`~/.claude/projects/**/*.jsonl`): what you did with it, and whether it is working right now.
//!
//! Claude Code appends an `assistant` entry for every reply (its `stop_reason` is "tool_use" while it wants to run a tool
//! and "end_turn" when it is done) and a `user` entry for the prompt and for each tool result. So a log whose last
//! conversation entry is anything but a finished reply is a turn in progress. It is *waiting for you* when a tool call has
//! no result and that tool is one that only waits on a person: `AskUserQuestion` (it asked you something) and
//! `ExitPlanMode` (it wants your approval of a plan) at once, and a tool that normally returns in milliseconds (`Read`,
//! `Edit`, `Write`...) that has been pending for a while, which means it is waiting for you to allow it. A long `Bash`
//! command looks the same from the log as a permission prompt, so it is never taken for waiting.

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::Value;

use super::executables::home_dir;
use super::logs::{collect, epoch, scan_state, tail_lines, EventCache, SummaryCache, TOOL_MAX_AGE, TURN_MAX_AGE, WAIT_MAX_AGE};
use crate::core::activity::{intern, project_label, summarize, Ev, Summary};
use crate::core::ports::AgentState;
use crate::core::sessions::SessionState;

/// An instant tool still pending after this many seconds is waiting for you to allow it.
pub const PERMISSION_WAIT: f64 = 20.0;

pub fn claude_dir() -> PathBuf {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home_dir().join(".claude"));
    base.join("projects")
}

static MODEL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^claude-(opus|sonnet|haiku)-(\d+)(?:-(\d{1,2}))?(?:\D|$)").unwrap());
static OLD_MODEL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^claude-(\d+)(?:-(\d+))?-(opus|sonnet|haiku)").unwrap());

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// "claude-opus-5-5" -> "Opus 5.5", "claude-sonnet-5" -> "Sonnet 5", older "claude-3-5-sonnet-2024..." -> "Sonnet 3.5";
/// any other name (Codex's) is shown as is.
pub fn model_label(model: Option<&str>) -> String {
    let Some(model) = model.filter(|m| !m.is_empty()) else { return "(unknown)".into() };
    if let Some(m) = MODEL_RE.captures(model) {
        let minor = m.get(3).map_or(String::new(), |x| format!(".{}", x.as_str()));
        return format!("{} {}{}", title_case(&m[1]), &m[2], minor);
    }
    if let Some(m) = OLD_MODEL_RE.captures(model) {
        let minor = m.get(2).map_or(String::new(), |x| format!(".{}", x.as_str()));
        return format!("{} {}{}", title_case(&m[3]), &m[1], minor);
    }
    model.to_string()
}

/// A JSON object, or nothing for any other kind of value (a log line may hold a plain string where an object is usual).
fn object_or_none<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct V<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
        type Value = Option<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("anything")
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
            T::deserialize(de::value::MapAccessDeserializer::new(map)).map(Some)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(None)
        }
        fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_none<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
    }
    d.deserialize_any(V(PhantomData))
}

/// Whether the key is there at all, whatever it holds (even null).
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    IgnoredAny::deserialize(d)?;
    Ok(true)
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    #[serde(default, deserialize_with = "present")]
    usage: bool,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    uuid: Option<String>,
    cwd: Option<String>,
    #[serde(default, deserialize_with = "object_or_none")]
    message: Option<Message>,
}

/// Every model call in one Claude Code log: an `assistant` entry with token usage. A reply written in several blocks
/// repeats its message id, so ids are what get counted. The session is the entry's own sessionId (sub-agents log under
/// their parent's). The file is streamed line by line: a month of logs is never held in memory.
pub fn read_events(path: &Path) -> io::Result<Vec<Ev>> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut reader = BufReader::new(File::open(path)?);
    let mut found = Vec::new();
    let mut raw = Vec::new();
    loop {
        raw.clear();
        if reader.read_until(b'\n', &mut raw)? == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&raw);
        if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Line>(&line) else { continue };
        let Some(message) = entry.message else { continue };
        if entry.kind.as_deref() != Some("assistant") || !message.usage || message.model.as_deref() == Some("<synthetic>") {
            continue; // (<synthetic>: Claude Code's own notices, not a model call)
        }
        let Some(when) = entry.timestamp.as_deref().and_then(epoch) else { continue };
        let session = entry.session_id.filter(|s| !s.is_empty()).unwrap_or_else(|| stem.clone());
        let id = message.id.filter(|s| !s.is_empty()).or(entry.uuid);
        found.push(Ev {
            when,
            session: intern(&session),
            id: id.as_deref().map(intern),
            project: intern(&project_label(entry.cwd.as_deref())),
            model: intern(&model_label(message.model.as_deref())),
        });
    }
    Ok(found)
}

static EVENTS: LazyLock<EventCache> = LazyLock::new(EventCache::default);

pub fn stats(now_epoch: f64, root: Option<&Path>) -> Option<Summary> {
    let root = root.map(Path::to_path_buf).unwrap_or_else(claude_dir);
    root.is_dir().then(|| summarize(&collect(&root, read_events, &EVENTS, now_epoch), now_epoch))
}

const FINISHED: [&str; 5] = ["end_turn", "stop_sequence", "max_tokens", "refusal", "pause_turn"]; // a reply that ends the turn
const INSTANT_TOOLS: [&str; 9] = ["Read", "Edit", "Write", "MultiEdit", "NotebookEdit", "Glob", "Grep", "LS", "TodoWrite"];

fn asks(tool: &str) -> Option<&'static str> {
    match tool {
        "AskUserQuestion" => Some("question"), // tools that exist to wait for you
        "ExitPlanMode" => Some("plan"),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    User,
    Assistant,
}

/// What a log's tail says, independent of the time of day.
#[derive(Debug)]
pub struct TailSummary {
    /// (time, kind, stop_reason) of the user/assistant entries, in order.
    pub turn: Vec<(f64, Kind, Option<String>)>,
    pub begin: f64,
    /// Tool call id -> (tool name, when it was called), until its result.
    pub pending: Vec<(String, String, f64)>,
    pub project: Option<String>,
    pub session: String,
}

pub fn summary(path: &Path) -> io::Result<Option<TailSummary>> {
    let mut turn = Vec::new();
    let mut pending: Vec<(String, String, f64)> = Vec::new();
    let (mut project, mut session) = (None, None);
    for line in tail_lines(path)? {
        if !line.contains("\"user\"") && !line.contains("\"assistant\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue };
        let kind = match entry.get("type").and_then(Value::as_str) {
            Some("user") => Kind::User,
            Some("assistant") => Kind::Assistant,
            _ => continue,
        };
        let Some(when) = entry.get("timestamp").and_then(Value::as_str).and_then(epoch) else { continue };
        let message = entry.get("message").filter(|m| m.is_object());
        turn.push((when, kind, message.and_then(|m| m.get("stop_reason")).and_then(Value::as_str).map(String::from)));
        if let Some(cwd) = entry.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
            project = Some(project_label(Some(cwd)));
        }
        if let Some(id) = entry.get("sessionId").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            session = Some(id.to_string()); // a sub-agent's log carries its parent session's id
        }
        let blocks = message.and_then(|m| m.get("content")).and_then(Value::as_array).cloned().unwrap_or_default();
        for block in blocks.iter().filter(|b| b.is_object()) {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    if let Some(id) = block.get("id").and_then(Value::as_str).filter(|i| !i.is_empty()) {
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                        match pending.iter_mut().find(|p| p.0 == id) {
                            Some(existing) => *existing = (id.to_string(), name, when),
                            None => pending.push((id.to_string(), name, when)),
                        }
                    }
                }
                Some("tool_result") => {
                    if let Some(id) = block.get("tool_use_id").and_then(Value::as_str) {
                        pending.retain(|p| p.0 != id);
                    }
                }
                _ => {}
            }
        }
    }
    if turn.is_empty() {
        return Ok(None);
    }
    let mut begin = 0;
    for (i, (_, kind, stop)) in turn.iter().enumerate() {
        // the turn began right after the last reply that finished one
        if *kind == Kind::Assistant && stop.as_deref().is_some_and(|s| FINISHED.contains(&s)) {
            begin = i + 1;
        }
    }
    let begin_time = turn[begin.min(turn.len() - 1)].0;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Some(TailSummary { turn, begin: begin_time, pending, project, session: session.unwrap_or(stem) }))
}

pub fn decide(summary: &TailSummary, now_epoch: f64) -> Option<SessionState> {
    let (last_time, last_kind, last_stop) = summary.turn.last()?;
    let age = now_epoch - last_time;
    let finished = *last_kind == Kind::Assistant && last_stop.as_deref().is_some_and(|s| FINISHED.contains(&s));
    let mut waiting_for: Option<String> = None;
    if !finished && age <= WAIT_MAX_AGE {
        for (_, name, when) in &summary.pending {
            if let Some(reason) = asks(name) {
                waiting_for = Some(reason.to_string());
                break;
            }
            if INSTANT_TOOLS.contains(&name.as_str()) && now_epoch - when >= PERMISSION_WAIT {
                waiting_for = Some(format!("approval ({name})"));
            }
        }
    }
    // None: still being written
    let running_tool = *last_kind == Kind::Assistant && matches!(last_stop.as_deref(), Some("tool_use") | None);
    let working = waiting_for.is_some() || (!finished && age <= if running_tool { TOOL_MAX_AGE } else { TURN_MAX_AGE });
    Some(SessionState {
        id: summary.session.clone(),
        working,
        waiting: waiting_for.is_some(),
        waiting_for,
        since: working.then_some(summary.begin),
        last: *last_time,
        project: summary.project.clone(),
    })
}

static SUMMARIES: LazyLock<SummaryCache<TailSummary>> = LazyLock::new(SummaryCache::default);

pub fn agent_state(now_epoch: f64, root: Option<&Path>) -> AgentState {
    let root = root.map(Path::to_path_buf).unwrap_or_else(claude_dir);
    scan_state(&root, summary, decide, &SUMMARIES, now_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn model_labels() {
        let l = |m| model_label(Some(m));
        assert_eq!(l("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(l("claude-sonnet-5"), "Sonnet 5");
        assert_eq!(l("claude-sonnet-5-5-20260101"), "Sonnet 5.5");
        assert_eq!(l("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(l("claude-3-5-sonnet-20241022"), "Sonnet 3.5");
        assert_eq!(l("claude-3-opus-20240229"), "Opus 3");
        assert_eq!(l("gpt-5-codex"), "gpt-5-codex");
        assert_eq!((model_label(None), model_label(Some(""))), ("(unknown)".into(), "(unknown)".into()));
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gm-claude-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn assistant(ts: &str, id: &str, stop: &str, content: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","sessionId":"S1","cwd":"C:\\Work\\Acme App","message":{{"id":"{id}","model":"claude-opus-5-5","stop_reason":{stop},"usage":{{"input_tokens":1}},"content":{content}}}}}"#
        )
    }

    #[test]
    fn events_count_replies_not_blocks_and_skip_noise() {
        let dir = temp("events");
        let log = [
            assistant("2026-10-02T10:00:00Z", "m1", "null", "[]"),
            assistant("2026-10-02T10:00:01Z", "m1", "\"end_turn\"", "[]"), // the same reply, second block
            assistant("2026-10-02T10:05:00Z", "m2", "\"end_turn\"", "[]"),
            r#"{"type":"assistant","timestamp":"2026-10-02T10:06:00Z","message":{"id":"m3","model":"<synthetic>","usage":{}}}"#.into(),
            r#"{"type":"user","timestamp":"2026-10-02T10:07:00Z","message":"a \"usage\" \"assistant\" mention"}"#.into(),
            "not json but has \"usage\" and \"assistant\"".into(),
        ]
        .join("\n");
        let path = dir.join("log.jsonl");
        fs::write(&path, log).unwrap();
        let events = read_events(&path).unwrap();
        assert_eq!(events.len(), 3, "the synthetic one, the user line and the junk are not calls");
        assert_eq!((&*events[0].project, &*events[0].model, &*events[0].session), ("Acme App", "Opus 5.5", "S1"));
        let summary = summarize(&events, epoch("2026-10-02T12:00:00Z").unwrap());
        assert_eq!(summary.d7.requests, 2, "m1 counted once");
        fs::remove_dir_all(&dir).unwrap();
    }

    fn write_log(dir: &Path, lines: &[String]) {
        fs::write(dir.join("s.jsonl"), lines.join("\n")).unwrap();
    }

    fn fresh_dir(name: &str) -> PathBuf {
        let dir = temp(name);
        let sub = dir.join("proj");
        fs::create_dir_all(&sub).unwrap();
        sub
    }

    #[test]
    fn a_finished_reply_is_idle_and_a_reply_in_progress_is_working() {
        let dir = fresh_dir("idle");
        // (the file's own modification time is "now", so ask about a moment close to the log's)
        write_log(&dir, &[assistant("2026-10-02T10:00:00Z", "m1", "\"end_turn\"", "[]")]);
        let s = decide(&summary(&dir.join("s.jsonl")).unwrap().unwrap(), epoch("2026-10-02T10:00:30Z").unwrap()).unwrap();
        assert_eq!((s.working, s.waiting, s.project.as_deref()), (false, false, Some("Acme App")));
        write_log(&dir, &[assistant("2026-10-02T10:00:00Z", "m1", "\"tool_use\"", r#"[{"type":"tool_use","id":"t1","name":"Bash"}]"#)]);
        let sum = summary(&dir.join("s.jsonl")).unwrap().unwrap();
        let s = decide(&sum, epoch("2026-10-02T10:05:00Z").unwrap()).unwrap();
        assert!(s.working && !s.waiting, "a long Bash is never taken for waiting");
        assert_eq!(s.since, Some(epoch("2026-10-02T10:00:00Z").unwrap()));
        let gone = decide(&sum, epoch("2026-10-02T10:11:00Z").unwrap()).unwrap();
        assert!(!gone.working, "quiet for longer than a tool may run: abandoned");
        fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn questions_plans_and_pending_permissions_mean_waiting() {
        let dir = fresh_dir("wait");
        let ask = |tool: &str| assistant("2026-10-02T10:00:00Z", "m1", "\"tool_use\"", &format!(r#"[{{"type":"tool_use","id":"t1","name":"{tool}"}}]"#));
        let at = |secs: &str| epoch(secs).unwrap();
        write_log(&dir, &[ask("AskUserQuestion")]);
        let sum = summary(&dir.join("s.jsonl")).unwrap().unwrap();
        assert_eq!(decide(&sum, at("2026-10-02T10:00:01Z")).unwrap().waiting_for.as_deref(), Some("question"));
        write_log(&dir, &[ask("ExitPlanMode")]);
        let sum = summary(&dir.join("s.jsonl")).unwrap().unwrap();
        assert_eq!(decide(&sum, at("2026-10-02T10:00:01Z")).unwrap().waiting_for.as_deref(), Some("plan"));
        write_log(&dir, &[ask("Edit")]);
        let sum = summary(&dir.join("s.jsonl")).unwrap().unwrap();
        assert_eq!(decide(&sum, at("2026-10-02T10:00:05Z")).unwrap().waiting_for, None, "an Edit that is only just pending");
        assert_eq!(decide(&sum, at("2026-10-02T10:00:25Z")).unwrap().waiting_for.as_deref(), Some("approval (Edit)"));
        let answered = [ask("Edit"), r#"{"type":"user","timestamp":"2026-10-02T10:00:10Z","message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#.into()];
        write_log(&dir, &answered);
        let sum = summary(&dir.join("s.jsonl")).unwrap().unwrap();
        assert_eq!(decide(&sum, at("2026-10-02T10:00:25Z")).unwrap().waiting_for, None, "its result came back");
        fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_folder_scan_lists_sessions_and_idle_is_the_default() {
        let root = temp("scan");
        assert!(!agent_state(1.0, Some(&root)).working);
        let sub = root.join("p");
        fs::create_dir_all(&sub).unwrap();
        let now = crate::util::epoch();
        let ts = chrono::DateTime::from_timestamp(now as i64 - 5, 0).unwrap().to_rfc3339();
        write_log(&sub, &[assistant(&ts, "m1", "\"tool_use\"", r#"[{"type":"tool_use","id":"t1","name":"Bash"}]"#)]);
        let s = agent_state(now, Some(&root));
        assert!(s.working && s.sessions.len() == 1 && s.sessions[0].id == "S1");
        fs::remove_dir_all(&root).unwrap();
    }
}
