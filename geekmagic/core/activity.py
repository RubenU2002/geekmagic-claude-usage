"""Activity statistics, computed the same way for every provider so they can be compared.

Every provider keeps a local log of what you did with it. Each model call is one *request*, a *session* is a
conversation, and from the events a provider's log reader produces come: requests and sessions in the last 24 hours and
7 days, requests per day for the last week, which projects and models those requests went to, and at what hours of the
day you work. (For Claude the totals agree with what `claude /usage` reports as "Last 24h / Last 7d".)

This is the pure part: events in, numbers out. Reading the logs is each provider's own business.
"""

from __future__ import annotations

import re
from collections import Counter
from datetime import datetime, timedelta

DAYS = 7  # days in the chart: today and the six before it


HOURS_DAYS = 28  # days the hours-of-the-day pattern looks back over (four full weeks)


TOP = 6  # projects / models kept per provider (the screens show the top few)


Event = tuple  # (epoch seconds, session key, unique id of the request or None, project, model)


GENERIC_FOLDERS = {"src", "app", "apps", "frontend", "backend", "server", "client", "web", "api", "lib", "packages",
                   "docs", "test", "tests", "scripts", "public", "dist", "build"}  # say nothing without their parent


def project_label(cwd: str | None) -> str:
    """The folder a session ran in, by name: "C:\\Work\\Acme App" -> "Acme App" (either kind of slash, and
    Windows drive letters in any case don't matter). A folder like `frontend` or `src` is shown with its parent
    ("Storefront/frontend"), as on its own it could be any project's."""
    parts = [p for p in re.split(r"[\\/]", cwd or "") if p]
    if not parts:
        return "(unknown)"
    if parts[-1].lower() in GENERIC_FOLDERS and len(parts) > 1 and not parts[-2].endswith(":"):
        return f"{parts[-2]}/{parts[-1]}"
    return parts[-1]


def start_of_day(epoch: float, days_ago: int) -> float:
    day = datetime.fromtimestamp(epoch).date() - timedelta(days=days_ago)
    return datetime(day.year, day.month, day.day).timestamp()


def _top(counter: Counter) -> list[dict]:
    return [{"name": name, "requests": n} for name, n in counter.most_common(TOP)]


def summarize(events: list[Event], now_epoch: float) -> dict:
    """What the screens need, from a provider's events:
    {"24h": {"requests", "sessions"}, "7d": {...}, "days": [{"date", "requests"}] oldest first (7 entries),
     "projects" / "models": [{"name", "requests"}] busiest first over the last 7 days, "total": requests those
     shares are of, "hours": requests per hour of the day (24 numbers) over the last HOURS_DAYS days}."""
    seen: set = set()
    unique: list[Event] = []
    for event in sorted(events):
        key = (event[1], event[2]) if event[2] is not None else (event[0], event[1], len(unique))
        if key not in seen:
            seen.add(key)
            unique.append(event)
    windows = {"24h": 24 * 3600, "7d": 7 * 24 * 3600}
    summary: dict = {}
    for label, span in windows.items():
        inside = [e for e in unique if now_epoch - e[0] <= span]
        summary[label] = {"requests": len(inside), "sessions": len({e[1] for e in inside})}
    today = datetime.fromtimestamp(now_epoch).date()
    counts = {today - timedelta(days=n): 0 for n in range(DAYS - 1, -1, -1)}
    hours = [0] * 24
    hours_since = start_of_day(now_epoch, HOURS_DAYS - 1)
    for when, *_ in unique:
        moment = datetime.fromtimestamp(when)
        if moment.date() in counts:
            counts[moment.date()] += 1
        if when >= hours_since:
            hours[moment.hour] += 1
    summary["days"] = [{"date": day.isoformat(), "requests": n} for day, n in counts.items()]
    week = [e for e in unique if now_epoch - e[0] <= windows["7d"]]
    summary["projects"] = _top(Counter(e[3] for e in week))
    summary["models"] = _top(Counter(e[4] for e in week))
    summary["total"] = len(week)
    summary["hours"] = hours
    return summary


# --- text for the screen ------------------------------------------------------------------------------

def counts_text(label: str, counts: dict) -> str:
    """"24h · 375 requests · 4 sessions" (singular for 1, thousands separators)."""
    r, s = counts["requests"], counts["sessions"]
    return f"{label} · {r:,} request{'' if r == 1 else 's'} · {s} session{'' if s == 1 else 's'}"


def weekday(date_text: str) -> str:
    """"2026-10-02" -> "Fr" (English, whatever the system language)."""
    return ("Mo", "Tu", "We", "Th", "Fr", "Sa", "Su")[datetime.fromisoformat(date_text).weekday()]


def hour12(hour: int) -> str:
    """14 -> "2 PM", 0 -> "12 AM"."""
    return f"{hour % 12 or 12} {'AM' if hour % 24 < 12 else 'PM'}"


def busiest_hours(hours: list[int], span: int = 2) -> tuple[int, int] | None:
    """The `span` consecutive hours (wrapping past midnight) with the most requests, as (first hour, hour after the
    last); None if there's no activity at all."""
    if not any(hours):
        return None
    # ties: the stretch that starts on its busier hour (not one that starts with an empty hour), then the earlier one
    best = max(range(24), key=lambda start: (sum(hours[(start + i) % 24] for i in range(span)), hours[start], -start))
    return best, (best + span) % 24


def hours_text(window: tuple[int, int]) -> str:
    """(14, 16) -> "2-4 PM"; across noon or midnight both ends say so: (11, 13) -> "11 AM-1 PM"."""
    start, end = window
    a, b = hour12(start), hour12(end)
    return f"{a.split()[0]}-{b}" if a.split()[1] == b.split()[1] else f"{a}-{b}"


def shares(items: list[dict], total: int, limit: int = 3) -> list[tuple[str, float]]:
    """The first `limit` of `items` as (name, share of `total` in 0..1), for the breakdown bars."""
    return [(item["name"], item["requests"] / total) for item in items[:limit]] if total else []
