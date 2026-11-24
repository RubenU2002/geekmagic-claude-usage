//! Activity statistics, computed the same way for every provider so they can be compared.
//!
//! Every provider keeps a local log of what you did with it. Each model call is one *request*, a *session* is a
//! conversation, and from the events a provider's log reader produces come: requests and sessions in the last 24 hours
//! and 7 days, requests per day for the last week, which projects and models those requests went to, and at what hours
//! of the day you work.
//!
//! This is the pure part: events in, numbers out. Reading the logs is each provider's own business.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};

use crate::util::LockExt;

pub const DAYS: usize = 7; // days in the chart: today and the six before it
pub const HOURS_DAYS: usize = 28; // days the hours-of-the-day pattern looks back over (four full weeks)
pub const TOP: usize = 6; // projects / models kept per provider

/// Folders that say nothing without their parent.
const GENERIC_FOLDERS: [&str; 18] = [
    "src", "app", "apps", "frontend", "backend", "server", "client", "web", "api", "lib", "packages", "docs", "test",
    "tests", "scripts", "public", "dist", "build",
];

/// One request found in a log. The strings are shared (interned): a month of logs repeats a handful of names.
#[derive(Debug, Clone)]
pub struct Ev {
    /// Epoch seconds.
    pub when: f64,
    /// The conversation it belongs to.
    pub session: Arc<str>,
    /// Unique id of the request, when the log has one.
    pub id: Option<Arc<str>>,
    pub project: Arc<str>,
    pub model: Arc<str>,
}

/// A shared copy of `text`: equal strings are stored once for the whole process.
pub fn intern(text: &str) -> Arc<str> {
    static POOL: OnceLock<Mutex<HashSet<Arc<str>>>> = OnceLock::new();
    let mut pool = POOL.get_or_init(Default::default).lk();
    if let Some(found) = pool.get(text) {
        return found.clone();
    }
    let new: Arc<str> = Arc::from(text);
    pool.insert(new.clone());
    new
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub requests: usize,
    pub sessions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub name: String,
    pub requests: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayCount {
    pub date: String, // ISO "2026-10-02"
    pub requests: usize,
}

/// What the screens need, from a provider's events.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub h24: Counts,
    pub d7: Counts,
    /// Requests per day, oldest first (DAYS entries).
    pub days: Vec<DayCount>,
    /// Busiest first over the last 7 days.
    pub projects: Vec<Named>,
    pub models: Vec<Named>,
    /// Requests those shares are of.
    pub total: usize,
    /// Requests per hour of the day (24 numbers) over the last HOURS_DAYS days.
    pub hours: [usize; 24],
}

impl Summary {
    pub fn counts(&self, label: &str) -> Counts {
        if label == "24h" {
            self.h24
        } else {
            self.d7
        }
    }
}

/// The folder a session ran in, by name: "C:\Work\Acme App" -> "Acme App". A folder like `frontend` or `src` is shown
/// with its parent ("Storefront/frontend"), as on its own it could be any project's.
pub fn project_label(cwd: Option<&str>) -> String {
    let parts: Vec<&str> = cwd.unwrap_or("").split(['\\', '/']).filter(|p| !p.is_empty()).collect();
    let Some(last) = parts.last() else { return "(unknown)".into() };
    if parts.len() > 1 && GENERIC_FOLDERS.contains(&last.to_lowercase().as_str()) && !parts[parts.len() - 2].ends_with(':') {
        return format!("{}/{}", parts[parts.len() - 2], last);
    }
    (*last).to_string()
}

/// Local midnight, `days_ago` days before the day of `epoch`.
pub fn start_of_day(epoch: f64, days_ago: usize) -> f64 {
    let moment = Local.timestamp_opt(epoch as i64, 0).single().unwrap_or_else(Local::now);
    let day = moment.date_naive() - Duration::days(days_ago as i64);
    let midnight = day.and_hms_opt(0, 0, 0).unwrap_or_default();
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map_or(epoch - (days_ago as f64 + 1.0) * 86_400.0, |d| d.timestamp() as f64)
}

/// Counts in the order first seen, busiest first (ties keep the order they were first seen in).
fn top(counter: &[(Arc<str>, usize)]) -> Vec<Named> {
    let mut sorted: Vec<&(Arc<str>, usize)> = counter.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1)); // (stable)
    sorted.into_iter().take(TOP).map(|(name, n)| Named { name: name.to_string(), requests: *n }).collect()
}

fn tally(items: impl Iterator<Item = Arc<str>>) -> Vec<(Arc<str>, usize)> {
    let mut index: HashMap<Arc<str>, usize> = HashMap::new();
    let mut out: Vec<(Arc<str>, usize)> = Vec::new();
    for name in items {
        match index.get(&name) {
            Some(i) => out[*i].1 += 1,
            None => {
                index.insert(name.clone(), out.len());
                out.push((name, 1));
            }
        }
    }
    out
}

pub fn summarize(events: &[Ev], now_epoch: f64) -> Summary {
    let mut sorted: Vec<&Ev> = events.iter().collect();
    sorted.sort_by(|a, b| {
        a.when.total_cmp(&b.when).then_with(|| a.session.cmp(&b.session)).then_with(|| a.id.cmp(&b.id))
    });
    // The same request is logged more than once when a reply is written in several blocks: its id says so.
    let mut seen: HashSet<(Arc<str>, Arc<str>)> = HashSet::new();
    let unique: Vec<&Ev> = sorted
        .into_iter()
        .filter(|e| match &e.id {
            Some(id) => seen.insert((e.session.clone(), id.clone())),
            None => true,
        })
        .collect();

    let counts_within = |span: f64| {
        let inside: Vec<&&Ev> = unique.iter().filter(|e| now_epoch - e.when <= span).collect();
        let sessions: HashSet<&Arc<str>> = inside.iter().map(|e| &e.session).collect();
        Counts { requests: inside.len(), sessions: sessions.len() }
    };
    let week_span = 7.0 * 24.0 * 3600.0;
    let today = Local.timestamp_opt(now_epoch as i64, 0).single().unwrap_or_else(Local::now).date_naive();
    let mut counts: Vec<(NaiveDate, usize)> =
        (0..DAYS).rev().map(|n| (today - Duration::days(n as i64), 0)).collect();
    let mut hours = [0usize; 24];
    let hours_since = start_of_day(now_epoch, HOURS_DAYS - 1);
    for e in &unique {
        let Some(moment) = Local.timestamp_opt(e.when as i64, 0).single() else { continue };
        if let Some(slot) = counts.iter_mut().find(|(d, _)| *d == moment.date_naive()) {
            slot.1 += 1;
        }
        if e.when >= hours_since {
            hours[moment.hour() as usize] += 1;
        }
    }
    let week: Vec<&&Ev> = unique.iter().filter(|e| now_epoch - e.when <= week_span).collect();
    Summary {
        h24: counts_within(24.0 * 3600.0),
        d7: counts_within(week_span),
        days: counts.into_iter().map(|(d, n)| DayCount { date: d.format("%Y-%m-%d").to_string(), requests: n }).collect(),
        projects: top(&tally(week.iter().map(|e| e.project.clone()))),
        models: top(&tally(week.iter().map(|e| e.model.clone()))),
        total: week.len(),
        hours,
    }
}

// --- text for the screen --------------------------------------------------------------------------------

/// 12345 -> "12,345".
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "24h · 375 requests · 4 sessions" (singular for 1, thousands separators).
pub fn counts_text(label: &str, counts: Counts) -> String {
    let (r, s) = (counts.requests, counts.sessions);
    format!(
        "{label} · {} request{} · {s} session{}",
        thousands(r),
        if r == 1 { "" } else { "s" },
        if s == 1 { "" } else { "s" }
    )
}

/// "2026-10-02" -> "Fr" (English, whatever the system language).
pub fn weekday(date_text: &str) -> &'static str {
    let names = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
    NaiveDate::parse_from_str(date_text, "%Y-%m-%d").map_or("--", |d| names[d.weekday().num_days_from_monday() as usize])
}

/// 14 -> "2 PM", 0 -> "12 AM".
pub fn hour12(hour: usize) -> String {
    format!("{} {}", if hour % 12 == 0 { 12 } else { hour % 12 }, if hour % 24 < 12 { "AM" } else { "PM" })
}

/// The `span` consecutive hours (wrapping past midnight) with the most requests, as (first hour, hour after the last);
/// None if there's no activity at all.
pub fn busiest_hours(hours: &[usize; 24], span: usize) -> Option<(usize, usize)> {
    if hours.iter().all(|h| *h == 0) {
        return None;
    }
    // ties: the stretch that starts on its busier hour (not one that starts with an empty hour), then the earlier one
    let key = |start: usize| (0..span).map(|i| hours[(start + i) % 24]).sum::<usize>();
    let best = (0..24usize).max_by(|a, b| {
        key(*a).cmp(&key(*b)).then(hours[*a].cmp(&hours[*b])).then(b.cmp(a))
    })?;
    Some((best, (best + span) % 24))
}

/// (14, 16) -> "2-4 PM"; across noon or midnight both ends say so: (11, 13) -> "11 AM-1 PM".
pub fn hours_text(window: (usize, usize)) -> String {
    let (a, b) = (hour12(window.0), hour12(window.1));
    let (a_num, a_ap) = a.split_once(' ').unwrap_or((&a, ""));
    let (_, b_ap) = b.split_once(' ').unwrap_or((&b, ""));
    if a_ap == b_ap {
        format!("{a_num}-{b}")
    } else {
        format!("{a}-{b}")
    }
}

/// The first `limit` of `items` as (name, share of `total` in 0..1), for the breakdown bars.
pub fn shares(items: &[Named], total: usize, limit: usize) -> Vec<(String, f64)> {
    if total == 0 {
        return Vec::new();
    }
    items.iter().take(limit).map(|i| (i.name.clone(), i.requests as f64 / total as f64)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(when: f64, session: &str, id: Option<&str>, project: &str, model: &str) -> Ev {
        Ev { when, session: intern(session), id: id.map(intern), project: intern(project), model: intern(model) }
    }

    #[test]
    fn project_labels() {
        assert_eq!(project_label(Some("C:\\Work\\Acme App")), "Acme App");
        assert_eq!(project_label(Some("/home/me/proj/")), "proj");
        assert_eq!(project_label(Some("/home/me/Storefront/frontend")), "Storefront/frontend");
        assert_eq!(project_label(Some("C:\\src")), "src");
        assert_eq!(project_label(Some("")), "(unknown)");
        assert_eq!(project_label(None), "(unknown)");
    }

    #[test]
    fn summary_counts_dedups_and_groups() {
        let now = Local.with_ymd_and_hms(2026, 10, 2, 12, 0, 0).unwrap().timestamp() as f64;
        let events = vec![
            ev(now - 100.0, "s1", Some("m1"), "A", "Opus 5"),
            ev(now - 90.0, "s1", Some("m1"), "A", "Opus 5"), // the same reply logged twice
            ev(now - 80.0, "s1", Some("m2"), "A", "Opus 5"),
            ev(now - 3.0 * 86_400.0, "s2", Some("m3"), "B", "Sonnet 5"),
            ev(now - 20.0 * 86_400.0, "s3", None, "C", "Haiku 5"),
            ev(now - 20.0 * 86_400.0 + 1.0, "s3", None, "C", "Haiku 5"), // no id: both count
        ];
        let s = summarize(&events, now);
        assert_eq!(s.h24, Counts { requests: 2, sessions: 1 });
        assert_eq!(s.d7, Counts { requests: 3, sessions: 2 });
        assert_eq!(s.days.len(), 7);
        assert_eq!(s.days.last().unwrap(), &DayCount { date: "2026-10-02".into(), requests: 2 });
        assert_eq!(s.projects, vec![Named { name: "A".into(), requests: 2 }, Named { name: "B".into(), requests: 1 }]);
        assert_eq!(s.total, 3);
        assert_eq!(s.hours.iter().sum::<usize>(), 5); // the 20-day-old ones are inside the 28-day hours window
        assert_eq!(s.hours[11], 2);
    }

    #[test]
    fn texts() {
        assert_eq!(counts_text("24h", Counts { requests: 375, sessions: 4 }), "24h · 375 requests · 4 sessions");
        assert_eq!(counts_text("7d", Counts { requests: 1, sessions: 1 }), "7d · 1 request · 1 session");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(weekday("2026-10-02"), "Fr");
        assert_eq!((hour12(14), hour12(0), hour12(12), hour12(23)), ("2 PM".into(), "12 AM".into(), "12 PM".into(), "11 PM".into()));
        assert_eq!(hours_text((14, 16)), "2-4 PM");
        assert_eq!(hours_text((11, 13)), "11 AM-1 PM");
        assert_eq!(hours_text((23, 1)), "11 PM-1 AM");
    }

    #[test]
    fn busiest_stretch() {
        let mut hours = [0usize; 24];
        assert_eq!(busiest_hours(&hours, 2), None);
        hours[14] = 5;
        hours[15] = 5;
        hours[3] = 9;
        assert_eq!(busiest_hours(&hours, 2), Some((14, 16)));
        let mut wrap = [0usize; 24];
        wrap[23] = 4;
        wrap[0] = 4;
        assert_eq!(busiest_hours(&wrap, 2), Some((23, 1)));
        let mut tie = [0usize; 24];
        tie[10] = 3; // (9,10) and (10,11) tie: the one that starts on its busier hour wins
        assert_eq!(busiest_hours(&tie, 2), Some((10, 12)));
    }

    #[test]
    fn shares_are_of_the_total() {
        let items = vec![Named { name: "a".into(), requests: 3 }, Named { name: "b".into(), requests: 1 }];
        assert_eq!(shares(&items, 4, 3), vec![("a".to_string(), 0.75), ("b".to_string(), 0.25)]);
        assert!(shares(&items, 0, 3).is_empty());
    }
}
