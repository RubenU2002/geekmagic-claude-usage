//! Several sessions of one agent at once (you may be working in several projects): merging and describing them.

use std::collections::HashMap;

/// What one log (or one merged session) says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionState {
    pub id: String,
    pub working: bool,
    pub waiting: bool,
    pub waiting_for: Option<String>,
    pub since: Option<f64>,
    pub last: f64,
    pub project: Option<String>,
}

/// One entry per session (a session's sub-agent logs are merged into it): working if any of its logs is, waiting if any
/// is, the project of its freshest log.
pub fn merge_sessions(states: &[SessionState]) -> Vec<SessionState> {
    let mut sorted: Vec<&SessionState> = states.iter().collect();
    sorted.sort_by(|a, b| a.last.total_cmp(&b.last)); // oldest first, so the freshest log's project wins
    let mut order: Vec<String> = Vec::new();
    let mut merged: HashMap<String, SessionState> = HashMap::new();
    for state in sorted {
        let mine = merged.entry(state.id.clone()).or_insert_with(|| {
            order.push(state.id.clone());
            SessionState { working: false, waiting: false, waiting_for: None, since: None, ..state.clone() }
        });
        mine.last = mine.last.max(state.last);
        if state.project.as_deref().is_some_and(|p| !p.is_empty()) {
            mine.project = state.project.clone();
        }
        if state.working {
            mine.working = true;
            mine.since = match (mine.since, state.since) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        if state.waiting {
            mine.waiting = true;
            mine.waiting_for = state.waiting_for.clone();
        }
    }
    order.into_iter().filter_map(|id| merged.remove(&id)).collect()
}

/// "45 s", "4 min", "1 h 05 min".
pub fn duration_text(seconds: f64) -> String {
    let seconds = crate::util::pyround(seconds).max(0);
    if seconds < 60 {
        return format!("{seconds} s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        format!("{minutes} min")
    } else {
        format!("{} h {:02} min", minutes / 60, minutes % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(id: &str, last: f64, project: Option<&str>, working: bool, waiting: bool, wf: Option<&str>, since: Option<f64>) -> SessionState {
        SessionState { id: id.into(), working, waiting, waiting_for: wf.map(Into::into), since, last, project: project.map(Into::into) }
    }

    #[test]
    fn a_sub_agent_log_is_merged_into_its_session() {
        let m = merge_sessions(&[
            st("S1", 10.0, Some("A"), true, false, None, Some(5.0)),
            st("S1", 90.0, Some("A"), true, false, None, Some(60.0)),
        ]);
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].working, m[0].since, m[0].last), (true, Some(5.0), 90.0));
    }

    #[test]
    fn waits_if_any_log_waits_and_idle_has_no_reason() {
        let m = merge_sessions(&[
            st("S1", 10.0, None, false, false, None, None),
            st("S1", 90.0, None, true, true, Some("question"), Some(40.0)),
        ]);
        assert_eq!((m[0].waiting, m[0].waiting_for.as_deref()), (true, Some("question")));
        let idle = merge_sessions(&[st("S1", 10.0, None, false, false, Some("left over"), Some(3.0))]);
        assert_eq!((idle[0].working, idle[0].waiting, idle[0].waiting_for.clone(), idle[0].since), (false, false, None, None));
    }

    #[test]
    fn freshest_log_names_the_project_and_unknown_does_not_erase() {
        let fresh = merge_sessions(&[
            st("S1", 90.0, Some("Fresh"), false, false, None, None),
            st("S1", 10.0, Some("Old"), false, false, None, None),
        ]);
        assert_eq!(fresh[0].project.as_deref(), Some("Fresh"));
        let kept = merge_sessions(&[
            st("S1", 10.0, Some("Alpha"), false, false, None, None),
            st("S1", 90.0, None, false, false, None, None),
        ]);
        assert_eq!(kept[0].project.as_deref(), Some("Alpha"));
        let two = merge_sessions(&[
            st("S1", 10.0, Some("same"), false, false, None, None),
            st("S2", 20.0, Some("same"), false, false, None, None),
        ]);
        assert_eq!(two.len(), 2);
        assert!(merge_sessions(&[]).is_empty());
    }

    #[test]
    fn durations() {
        let t: Vec<String> = [0.0, 5.0, 59.6, 60.0, 240.0, 3599.0, 3600.0, 3900.0, -4.0].map(duration_text).to_vec();
        assert_eq!(t, ["0 s", "5 s", "1 min", "1 min", "4 min", "59 min", "1 h 00 min", "1 h 05 min", "0 s"]);
    }
}
