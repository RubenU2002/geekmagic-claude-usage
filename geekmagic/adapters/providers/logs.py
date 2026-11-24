"""Reading the providers' local logs: the parts every provider's reader shares.

A cache so a finished log is read once, a scan of a folder for the logs recent enough to matter, and the "is an agent
working" decision over a folder of logs. What a log *says* is each provider's own business (see claude_logs, codex_logs).
"""

from __future__ import annotations

from datetime import datetime
from pathlib import Path

from geekmagic.core.activity import DAYS, HOURS_DAYS, Event, start_of_day
from geekmagic.core.sessions import merge_sessions

SCAN_AGE = 3 * 3600  # logs older than this can't belong to a turn that is still being worked on or waited for
TURN_MAX_AGE = 120  # seconds of silence after which a turn that isn't running a tool counts as abandoned
TOOL_MAX_AGE = 600  # ...but while a tool runs (a long build) it may be quiet this long
WAIT_MAX_AGE = 3 * 3600  # a question or approval left unanswered this long is taken for a forgotten session
TAIL_BYTES = 262_144  # how much of the end of a log is read

_event_cache: dict[str, tuple[float, int, list[Event]]] = {}  # path -> (mtime, size, events)
_summary_cache: dict[str, tuple[float, int, dict | None]] = {}  # path -> (mtime, size, summary of its tail)


def epoch(text: str | None) -> float | None:
    """An ISO timestamp as epoch seconds, None if it isn't one."""
    try:
        return datetime.fromisoformat((text or "").replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def tail_lines(path: Path) -> list[str]:
    with path.open("rb") as f:
        f.seek(0, 2)
        size = f.tell()
        f.seek(max(0, size - TAIL_BYTES))
        data = f.read().decode("utf-8", errors="replace").splitlines()
    return data[1:] if size > TAIL_BYTES else data  # the first line of a tail is usually cut in half


# --- activity: every request in the logs ---------------------------------------------------------------------------

def events(path: Path, mtime: float, size: int, reader) -> list[Event]:
    """A finished log never changes, so each is read once; only one that grew since last time is read again."""
    cached = _event_cache.get(str(path))
    if cached and cached[0] == mtime and cached[1] == size:
        return cached[2]
    try:
        found = reader(path)
    except OSError:
        return []
    _event_cache[str(path)] = (mtime, size, found)
    return found


def collect(root: Path, reader, now_epoch: float) -> list[Event]:
    """The events of every log under `root` recent enough to matter (the longest look-back is the hours pattern)."""
    oldest = start_of_day(now_epoch, max(DAYS, HOURS_DAYS) - 1)
    found: list[Event] = []
    for path in root.rglob("*.jsonl"):
        try:
            stat = path.stat()
        except OSError:
            continue
        if stat.st_mtime >= oldest:  # a log untouched since before then has nothing in it that matters
            found.extend(events(path, stat.st_mtime, stat.st_size, reader))
    return found


# --- agents: what is going on right now ----------------------------------------------------------------------------

def summary(path: Path, mtime: float, size: int, reader) -> dict | None:
    """What a log's tail says, independent of the time of day: a log that hasn't changed is only read once."""
    cached = _summary_cache.get(str(path))
    if cached and cached[0] == mtime and cached[1] == size:
        return cached[2]
    found = reader(path)
    _summary_cache[str(path)] = (mtime, size, found)
    return found


def scan_state(root: Path, reader, decide, now_epoch: float) -> dict:
    """The most telling log's state: a waiting one first (it needs you), else the working one that has been going the
    longest, else the freshest log's, plus "sessions" (every session seen, see merge_sessions). Always has the same keys."""
    idle = {"working": False, "waiting": False, "waiting_for": None, "since": None, "last": None, "project": None,
            "sessions": []}
    if not root.is_dir():
        return idle
    states = []
    for path in root.rglob("*.jsonl"):
        try:
            stat = path.stat()
            if now_epoch - stat.st_mtime > SCAN_AGE:
                continue
            state = decide(summary(path, stat.st_mtime, stat.st_size, reader), now_epoch)
        except OSError:
            continue
        if state:
            states.append(state)
    if not states:
        return idle
    waiting = [s for s in states if s["waiting"]]
    working = [s for s in states if s["working"]]
    if waiting:
        top = max(waiting, key=lambda s: s["last"])
    elif working:
        top = min(working, key=lambda s: s["since"] or now_epoch)
    else:
        top = max(states, key=lambda s: s["last"])
    return {**{k: v for k, v in top.items() if k != "id"}, "sessions": merge_sessions(states)}
