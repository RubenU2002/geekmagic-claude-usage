//! Every tunable of the tray app, in one place (seconds unless it says otherwise).

pub const SETTLE_SECONDS: f64 = 5.0; // quiet time after a click before the slow fetch + upload starts
pub const STALE_AFTER: f64 = 30.0; // a switched-to image uploaded more recently than this isn't refreshed again
pub const RETRY_SECONDS: f64 = 10.0; // how often to retry while the device is unreachable
pub const STALE_SECONDS: f64 = 180.0; // numbers that couldn't be refreshed for this long get the dimmed "no fresh data" image
pub const OTHER_REFRESH: f64 = 120.0; // how often the provider that isn't on screen is queried (for its alerts and fresh image)
pub const REUSE_WITHIN: f64 = 30.0; // an agent started or stopped: a reading this fresh is redrawn instead of read again
pub const REDISCOVER_AFTER: f64 = 15.0; // unreachable for this long -> look for the device on the network (a new IP?)
pub const REDISCOVER_EVERY: f64 = 30.0; // ...and then at most this often (a scan takes ~4 s)
pub const OFFLINE_NOTICE_AFTER: f64 = 45.0; // the screen has been gone this long (and a search found nothing): say so, once
pub const DISCOVER_RETRY: f64 = 30.0; // while there's no address at all, how often to scan
pub const LOCK_POLL: f64 = 3.0; // between checks of whether the computer got locked / unlocked
pub const PAUSE_POLL: f64 = 5.0; // while paused, how often the worker looks again
pub const LOGIN_COOLDOWN: f64 = 300.0; // a sign-in window isn't opened again for the same provider within this many seconds
pub const LOGIN_POLL: f64 = 10.0; // while one is being signed in, how often its usage is tried again
pub const LOCAL_STATS_EVERY: f64 = 600.0; // the providers' local activity counts are refreshed at most this often
pub const LOCAL_STATS_RETRY: f64 = 60.0; // ...or this often while one of them is still missing (it failed)
pub const AGENT_POLL: f64 = 2.0; // between looks at the agents' logs while one is working (a read of a few log tails)
pub const AGENT_IDLE_POLL: f64 = 30.0; // ...and while none is: a webhook looks at once, this only catches a missed one
pub const AGENT_IDLE_POLLS: u32 = 2; // polls in a row that must say "idle" before a run counts as finished (no flapping)
pub const AGENT_WAIT_POLLS: u32 = 2; // polls in a row that must say "waiting for you" before it's shown and notified
pub const AGENT_MIN_RUN: f64 = 60.0; // a run shorter than this ends without a notification
pub const BACKOFF_FROM: u32 = 3; // consecutive failed reads of a provider after which it is left alone for a while
pub const BACKOFF_BASE: f64 = 30.0; // ...first for this long, doubling each time it fails again
pub const BACKOFF_MAX: f64 = 120.0; // ...up to this long (a click or "Actualizar ahora" tries at once)
pub const HISTORY_MAX: usize = 150; // readings kept per usage window, for the recent-pace estimate
pub const DEFAULT_BRIGHTNESS_CHOICES: [i64; 6] = [100, 75, 50, 25, 10, 0]; // the menu's brightness levels (the device takes -10..100)
pub const LOCK_DIM_LEVEL: i64 = 0; // backlight level while the computer is locked, if "dim when locked" is on

/// How often a window's reading is added to the history at least (or when the % changed).
pub fn history_every(window: &str) -> f64 {
    if window == "current" {
        300.0
    } else {
        1800.0
    }
}

/// The device's own night schedule by default: 10 PM - 7 AM at 10 %.
pub const NIGHT_DEFAULT: (bool, i64, i64, i64) = (false, 22, 7, 10);

/// The webhook the app listens on, and the safety refresh between webhooks.
pub const WEBHOOK_PORT: u16 = 8765;
/// With no webhook at all, the screen is refreshed this often anyway (0 turns that off).
pub const FALLBACK_INTERVAL: f64 = 900.0;
