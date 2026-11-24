"""Usage thresholds: when a bar changes colour, and when to send a desktop notification."""

from __future__ import annotations

WARN_PCT = 70  # the bar and percentage turn yellow from here
CRIT_PCT = 90  # ... and red from here
NOTIFY_PCTS = (80, 95)  # notify the first time usage reaches each of these
RESET_DROP = 30  # usage falling by this many points can only mean the window reset
HYSTERESIS = 5  # points below a threshold before it can notify again


def bar_state(pct: float | None) -> str:
    """"ok", "warn" or "crit" for a usage percentage (None counts as ok)."""
    if pct is None:
        return "ok"
    if pct >= CRIT_PCT:
        return "crit"
    return "warn" if pct >= WARN_PCT else "ok"


def level(pct: float | None) -> int:
    """How many NOTIFY_PCTS `pct` has reached (0, 1 or 2)."""
    return 0 if pct is None else sum(pct >= t for t in NOTIFY_PCTS)


def evaluate(prev_level: int, prev_pct: float | None, pct: float | None) -> tuple[int, tuple[str, int | None] | None]:
    """Compare a new reading with the previous one.

    Returns (new_level, event); event is ("rise", threshold) when usage newly reached a notify threshold,
    ("reset", None) when it dropped far enough to mean the window reset after an alert was raised, else None.
    """
    if pct is None:
        return prev_level, None
    new = level(pct)
    if new > prev_level:
        return new, ("rise", NOTIFY_PCTS[new - 1])
    if new < prev_level:
        if prev_pct is not None and prev_pct - pct >= RESET_DROP:
            return new, ("reset", None)
        # a small dip below a threshold doesn't re-arm it, or readings hovering around it would keep notifying
        rearm_below = NOTIFY_PCTS[prev_level - 1] - HYSTERESIS
        return (new if pct < rearm_below else prev_level), None
    return new, None
