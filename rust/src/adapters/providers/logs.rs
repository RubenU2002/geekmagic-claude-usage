//! Reading the providers' local logs: the parts every provider's reader shares.
//!
//! A cache so a finished log is read once, a scan of a folder for the logs recent enough to matter, and the "is an agent
//! working" decision over a folder of logs. What a log *says* is each provider's own business (see claude_logs,
//! codex_logs).

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};

use crate::core::activity::{start_of_day, Ev, DAYS, HOURS_DAYS};
use crate::core::ports::AgentState;
use crate::core::sessions::{merge_sessions, SessionState};
use crate::util::LockExt;

pub const SCAN_AGE: f64 = 3.0 * 3600.0; // logs older than this can't belong to a turn still being worked on or waited for
pub const TURN_MAX_AGE: f64 = 120.0; // seconds of silence after which a turn that isn't running a tool counts as abandoned
pub const TOOL_MAX_AGE: f64 = 600.0; // ...but while a tool runs (a long build) it may be quiet this long
pub const WAIT_MAX_AGE: f64 = 3.0 * 3600.0; // a question or approval left unanswered this long is taken for a forgotten session
pub const TAIL_BYTES: u64 = 262_144; // how much of the end of a log is read

/// An ISO timestamp as epoch seconds, None if it isn't one.
pub fn epoch(text: &str) -> Option<f64> {
    let text = text.trim();
    if let Ok(d) = DateTime::parse_from_rfc3339(text) {
        return Some(d.timestamp_millis() as f64 / 1000.0);
    }
    let naive = NaiveDateTime::parse_from_str(&text.replace(' ', "T"), "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    Local.from_local_datetime(&naive).earliest().map(|d| d.timestamp_millis() as f64 / 1000.0)
}

/// The last TAIL_BYTES of a file as lines (the first line of a tail is usually cut in half, so it is dropped).
pub fn tail_lines(path: &Path) -> io::Result<Vec<String>> {
    let mut f = File::open(path)?;
    let size = f.metadata()?.len();
    f.seek(SeekFrom::Start(size.saturating_sub(TAIL_BYTES)))?;
    let mut data = Vec::new();
    f.take(TAIL_BYTES).read_to_end(&mut data)?;
    let text = String::from_utf8_lossy(&data);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if size > TAIL_BYTES && !lines.is_empty() {
        lines.remove(0);
    }
    Ok(lines)
}

/// Every `*.jsonl` file under `root` with its modification time (epoch seconds) and size.
pub fn walk_jsonl(root: &Path) -> Vec<(PathBuf, f64, u64)> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                if let Ok(meta) = entry.metadata() {
                    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs_f64());
                    found.push((path, mtime, meta.len()));
                }
            }
        }
    }
    found
}

// --- activity: every request in the logs -----------------------------------------------------------------

/// The events of each log read so far: a finished log never changes, so each is read once.
#[derive(Default)]
pub struct EventCache(Mutex<HashMap<PathBuf, (f64, u64, Arc<Vec<Ev>>)>>);

pub type EventReader = fn(&Path) -> io::Result<Vec<Ev>>;

/// The events of every log under `root` recent enough to matter (the longest look-back is the hours pattern).
pub fn collect(root: &Path, reader: EventReader, cache: &EventCache, now_epoch: f64) -> Vec<Ev> {
    let oldest = start_of_day(now_epoch, DAYS.max(HOURS_DAYS) - 1);
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for (path, mtime, size) in walk_jsonl(root) {
        if mtime < oldest {
            continue; // a log untouched since before then has nothing in it that matters
        }
        let cached = cache.0.lk().get(&path).filter(|(m, s, _)| *m == mtime && *s == size).map(|(_, _, e)| e.clone());
        let events = match cached {
            Some(events) => events,
            None => match reader(&path) {
                Ok(events) => {
                    let events = Arc::new(events);
                    cache.0.lk().insert(path.clone(), (mtime, size, events.clone()));
                    events
                }
                Err(_) => continue,
            },
        };
        found.extend(events.iter().cloned());
        seen.insert(path);
    }
    cache.0.lk().retain(|p, _| seen.contains(p)); // (logs that went away or aged out)
    found
}

// --- agents: what is going on right now ------------------------------------------------------------------

/// What each log's tail said last time it was read.
pub struct SummaryCache<S>(Mutex<HashMap<PathBuf, (f64, u64, Option<Arc<S>>)>>);

impl<S> Default for SummaryCache<S> {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

pub type SummaryReader<S> = fn(&Path) -> io::Result<Option<S>>;
pub type Decider<S> = fn(&S, f64) -> Option<SessionState>;

/// The most telling log's state: a waiting one first (it needs you), else the working one that has been going the
/// longest, else the freshest log's, plus every session seen (see `merge_sessions`).
pub fn scan_state<S>(root: &Path, reader: SummaryReader<S>, decide: Decider<S>, cache: &SummaryCache<S>, now_epoch: f64) -> AgentState {
    if !root.is_dir() {
        return AgentState::default();
    }
    let mut states = Vec::new();
    let mut seen = HashSet::new();
    for (path, mtime, size) in walk_jsonl(root) {
        if now_epoch - mtime > SCAN_AGE {
            continue;
        }
        let cached = cache.0.lk().get(&path).filter(|(m, s, _)| *m == mtime && *s == size).map(|(_, _, s)| s.clone());
        let summary = match cached {
            Some(summary) => summary,
            None => match reader(&path) {
                Ok(found) => {
                    let found = found.map(Arc::new);
                    cache.0.lk().insert(path.clone(), (mtime, size, found.clone()));
                    found
                }
                Err(_) => continue,
            },
        };
        seen.insert(path);
        if let Some(state) = summary.and_then(|s| decide(&s, now_epoch)) {
            states.push(state);
        }
    }
    cache.0.lk().retain(|p, _| seen.contains(p));
    if states.is_empty() {
        return AgentState::default();
    }
    let by_last = |a: &&SessionState, b: &&SessionState| a.last.total_cmp(&b.last);
    let waiting: Vec<&SessionState> = states.iter().filter(|s| s.waiting).collect();
    let working: Vec<&SessionState> = states.iter().filter(|s| s.working).collect();
    let top = if !waiting.is_empty() {
        waiting.into_iter().max_by(by_last)
    } else if !working.is_empty() {
        working.into_iter().min_by(|a, b| a.since.unwrap_or(now_epoch).total_cmp(&b.since.unwrap_or(now_epoch)))
    } else {
        states.iter().max_by(by_last)
    };
    let top = top.cloned().unwrap_or_default();
    AgentState {
        working: top.working,
        waiting: top.waiting,
        waiting_for: top.waiting_for,
        since: top.since,
        last: Some(top.last),
        project: top.project,
        sessions: merge_sessions(&states),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(epoch("2026-10-02T14:57:00Z"), Some(1_790_953_020.0));
        assert_eq!(epoch("2026-10-02T14:57:00.500+00:00"), Some(1_790_953_020.5));
        assert_eq!(epoch("nonsense"), None);
        assert_eq!(epoch(""), None);
    }

    #[test]
    fn tails_drop_the_cut_first_line_only_when_cut() {
        let dir = std::env::temp_dir().join(format!("gm-tail-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let small = dir.join("small.jsonl");
        fs::write(&small, "a\nb\nc\n").unwrap();
        assert_eq!(tail_lines(&small).unwrap(), ["a", "b", "c"]);
        let big = dir.join("big.jsonl");
        let line = format!("{}\n", "x".repeat(99));
        fs::write(&big, line.repeat(5000)).unwrap(); // 500 KB
        let tail = tail_lines(&big).unwrap();
        assert!(tail.len() < 5000 && tail.iter().all(|l| l.len() == 99), "the half line at the start is gone");
        fs::remove_dir_all(&dir).unwrap();
    }
}
