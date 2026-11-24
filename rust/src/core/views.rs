//! The views of two providers at once: their keys, and what the application needs to know about them.

pub const SPLIT: &str = "split"; // both providers on one screen
pub const STATS: &str = "stats"; // activity numbers for both providers
pub const BREAKDOWN: &str = "breakdown"; // which projects and models the requests went to
pub const HOURS: &str = "hours"; // at what hours of the day you work

pub const PANEL_KEYS: [&str; 4] = [SPLIT, STATS, BREAKDOWN, HOURS];

pub fn view_name(key: &str) -> &'static str {
    match key {
        SPLIT => "vista dividida",
        STATS => "estadísticas",
        BREAKDOWN => "proyectos y modelos",
        HOURS => "horas pico",
        _ => "",
    }
}

/// Drawn from the providers' local activity logs (counted in the background).
pub fn is_stats_view(key: &str) -> bool {
    matches!(key, STATS | BREAKDOWN | HOURS)
}

pub fn is_panel_key(key: &str) -> bool {
    PANEL_KEYS.contains(&key)
}
