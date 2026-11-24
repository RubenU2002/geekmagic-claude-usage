"""Pace projection: at the rate you're going, where will a usage window end up, and when would it run out?"""

from __future__ import annotations

from datetime import datetime

from geekmagic.core.model import WINDOWS, Projection, Usage

# Window lengths in minutes. Codex reports its own (windowDurationMins); Claude's /usage doesn't, so these apply.
DEFAULT_WINDOW_MIN = {"current": 300, "weekly": 10080}
MIN_ELAPSED = 0.08  # too early in a window to say anything useful (fraction of the window)
MIN_PCT = 2.0  # likewise when almost nothing has been used
LOOKBACK_MIN = {"current": 30, "weekly": 1440}  # how far back "your recent pace" looks (the week: the last day)
MIN_SPAN_MIN = 5  # recent readings must span at least this long to give a rate...
MIN_SPAN_SHARE = 0.25  # ...and at least this share of the look-back, so a short burst isn't stretched over days


def project(pct: float | None, reset_at: datetime | None, now: datetime, window_min: float,
            recent_rate: float | None = None) -> Projection | None:
    """Projection for one window, or None when it's too early / there's nothing to go on.

    The pace is the faster of your average since the window started and your recent rate (percentage points
    per minute), so a burst of work shows up instead of being averaged away.
    """
    if pct is None or reset_at is None or pct < MIN_PCT:
        return None
    remaining = (reset_at - now).total_seconds() / 60
    elapsed = window_min - remaining
    if remaining <= 0 or elapsed <= 0 or elapsed / window_min < MIN_ELAPSED:
        return None
    rate = max(pct / elapsed, recent_rate or 0.0)
    end = pct + rate * remaining
    hits = end >= 100 and pct < 100
    return Projection(rate=rate, end=end, hits_limit=hits, minutes_to_limit=(100 - pct) / rate if hits else None)


def recent_rate(history: list, reset_epoch: float | None, now_epoch: float, lookback_min: float) -> float | None:
    """Percentage points per minute over the last `lookback_min`, from [epoch, pct, reset_epoch] readings of the
    current window. None if there isn't enough history (or it was flat/falling)."""
    if reset_epoch is None:
        return None
    points = [(t, p) for t, p, r in history if r is not None and abs(r - reset_epoch) < 120 and now_epoch - t <= lookback_min * 60]
    if len(points) < 2:
        return None
    (t0, p0), (t1, p1) = min(points), max(points)
    span = (t1 - t0) / 60
    if span < max(MIN_SPAN_MIN, lookback_min * MIN_SPAN_SHARE) or p1 <= p0:
        return None
    return (p1 - p0) / span


def format_minutes(minutes: float) -> str:
    """Compact duration: "45m", "2h 10m", "3d 4h"."""
    minutes = max(0, round(minutes))
    days, rest = divmod(minutes, 1440)
    hours, mins = divmod(rest, 60)
    if days:
        return f"{days}d {hours}h"
    return f"{hours}h {mins}m" if hours else f"{mins}m"


def describe(proj: Projection | None) -> tuple[str, str] | None:
    """(text, state) for the screen: "Full in 1h 30m" (crit under an hour, else warn) or "~62% at reset" (muted)."""
    if not proj:
        return None
    if proj.hits_limit:
        return f"Full in {format_minutes(proj.minutes_to_limit)}", "crit" if proj.minutes_to_limit < 60 else "warn"
    return f"~{proj.end:.0f}% at reset", "ok"


def annotate(usage: Usage, recent: dict[str, float | None] | None = None) -> Usage:
    """Set usage.pace = {"current": projection|None, "weekly": projection|None} (in place) and return `usage`."""
    recent = recent or {}
    usage.pace = {
        kind: project(usage.window(kind).pct, usage.window(kind).reset, usage.now,
                      usage.window(kind).minutes or DEFAULT_WINDOW_MIN[kind], recent.get(kind))
        for kind in WINDOWS
    }
    return usage
