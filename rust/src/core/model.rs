//! What the app passes around: a provider's usage, as typed objects.

use std::sync::Arc;

use chrono::{DateTime, Local, Utc};

use super::activity::Summary;

/// The two usage windows every provider has, in the order the screens show them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowKind {
    Current,
    Weekly,
}

impl WindowKind {
    pub const ALL: [WindowKind; 2] = [WindowKind::Current, WindowKind::Weekly];

    pub fn key(self) -> &'static str {
        match self {
            WindowKind::Current => "current",
            WindowKind::Weekly => "weekly",
        }
    }

    /// How notifications name it.
    pub fn label(self) -> &'static str {
        match self {
            WindowKind::Current => "sesión",
            WindowKind::Weekly => "semana",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// One usage window: the 5-hour "session" (current) or the week.
#[derive(Debug, Clone, Default)]
pub struct Window {
    pub pct: Option<f64>,
    pub reset: Option<DateTime<Utc>>,
    /// How long the window is, when the provider says (Codex does); else a default is assumed.
    pub minutes: Option<f64>,
}

/// Where a window is heading at the pace you're going.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    pub rate: f64, // percentage points per minute
    pub end: f64,  // projected % at reset (may exceed 100)
    pub hits_limit: bool,
    pub minutes_to_limit: Option<f64>, // None if it won't run out
}

/// The free rate-limit resets Codex has granted.
#[derive(Debug, Clone, Default)]
pub struct CodexExtra {
    pub free_resets: i64,
    pub next_reset_expires_days: Option<i64>,
}

/// What the stats views show for one provider.
#[derive(Debug, Clone)]
pub enum Activity {
    /// Still counting.
    Counting,
    /// It has no logs.
    NoLogs,
    Stats(Arc<Summary>),
}

/// A provider's usage right now, plus what the screens need to know about it.
#[derive(Debug, Clone)]
pub struct Usage {
    /// "Claude": who it belongs to (it also picks its mascot, colours and animations).
    pub title: String,
    pub current: Window,
    pub weekly: Window,
    /// When it was read.
    pub now: DateTime<Local>,
    pub pace: [Option<Projection>; 2],
    pub codex_extra: Option<CodexExtra>,
    pub working: bool,
    pub waiting: bool,
    /// These numbers couldn't be refreshed for a while.
    pub stale: bool,
    pub activity: Activity,
}

impl Usage {
    pub fn new(title: &str, current: Window, weekly: Window, now: DateTime<Local>) -> Self {
        Self {
            title: title.to_string(),
            current,
            weekly,
            now,
            pace: [None, None],
            codex_extra: None,
            working: false,
            waiting: false,
            stale: false,
            activity: Activity::Counting,
        }
    }

    pub fn window(&self, kind: WindowKind) -> &Window {
        match kind {
            WindowKind::Current => &self.current,
            WindowKind::Weekly => &self.weekly,
        }
    }

    pub fn pace_of(&self, kind: WindowKind) -> Option<&Projection> {
        self.pace[kind.index()].as_ref()
    }
}

/// What a provider's screen says when it has never been read.
#[derive(Debug, Clone)]
pub struct ErrorScreen {
    pub title: String,
    pub message: String,
    pub now: DateTime<Local>,
    /// Signing in is what's needed (the heading says so).
    pub signin: bool,
}

/// What a screen is drawn from: one provider's reading, the readings of two, or why one can't be read.
#[derive(Debug, Clone)]
pub enum Data {
    One(Usage),
    Panels(Vec<Usage>),
    Error(ErrorScreen),
}
