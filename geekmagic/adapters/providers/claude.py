"""Claude Code: its usage (from its own built-in /usage, which costs nothing)."""

from __future__ import annotations

import json
import re
import subprocess
from datetime import datetime
from zoneinfo import ZoneInfo

from geekmagic.adapters.providers import claude_logs
from geekmagic.core import pace
from geekmagic.core.model import Usage, Window
from geekmagic.core.ports import Provider
from geekmagic.core.errors import SignInNeeded, UsageError
from geekmagic.adapters.providers.executables import which


_SESSION_RE = re.compile(r"^Current session:\s*(\d+(?:\.\d+)?)% used(?:\s*·\s*resets\s+(.+))?$", re.MULTILINE)


_WEEK_RE = re.compile(r"^Current week(?:\s*\([^)]*\))?:\s*(\d+(?:\.\d+)?)% used(?:\s*·\s*resets\s+(.+))?$", re.MULTILINE)


_RESET_RE = re.compile(
    r"(?P<month>[A-Za-z]{3})\s+(?P<day>\d{1,2})(?:,|\s+at)\s+"
    r"(?P<hour>\d{1,2})(?::(?P<minute>\d{2}))?(?P<meridiem>am|pm)\s+\((?P<tz>[^)]+)\)",
    re.IGNORECASE,
)


# What Claude Code says when it isn't signed in ("Not logged in · Please run /login"), however it phrases it.
_SIGNED_OUT_RE = re.compile(r"not logged in|/login|log ?in again|sign ?in|authenticat|invalid api key|unauthori[sz]ed|token .*expired", re.I)


TIMEOUT_SECONDS = 90  # it takes ~10 s, but 30 s or more on a slow connection: a timeout costs a whole cycle for nothing


def fetch_usage() -> Usage:
    """Call Claude Code's built-in /usage command. Guaranteed $0 cost."""
    try:
        proc = subprocess.run(
            [
                which("claude") or "claude", "-p", "--safe-mode",
                "--output-format", "json",
                "--max-budget-usd", "0.000001",
                "--tools", "",
                "--no-session-persistence",
                "--no-chrome",
                "/usage",
            ],
            capture_output=True, text=True, encoding="utf-8", timeout=TIMEOUT_SECONDS,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
    except FileNotFoundError as e:
        raise UsageError("`claude` CLI not found in PATH. Install/open Claude Code first.") from e
    except subprocess.TimeoutExpired as e:
        raise UsageError("Timed out waiting for `claude /usage`.") from e

    if proc.returncode != 0:
        if _SIGNED_OUT_RE.search(f"{proc.stdout} {proc.stderr}"):
            raise SignInNeeded("Claude Code isn't signed in.")
        raise UsageError(f"claude exited {proc.returncode}: {proc.stderr.strip()}")

    payload = json.loads(proc.stdout)

    cost = payload.get("total_cost_usd", 0)
    if float(cost) != 0:
        raise UsageError("Refusing result: /usage unexpectedly incurred cost.")

    result = payload.get("result")
    if payload.get("is_error") or not isinstance(result, str):
        if _SIGNED_OUT_RE.search(str(result)):
            raise SignInNeeded("Claude Code isn't signed in.")
        raise UsageError("Claude Code did not return usage text.")
    return pace.annotate(_parse(result))


def _parse(text: str) -> Usage:
    now = datetime.now().astimezone()
    session = _SESSION_RE.search(text)
    week = _WEEK_RE.search(text)
    if not session and not week:
        raise UsageError("Could not find usage lines in /usage output:\n" + text)
    return Usage(
        title="Claude",
        current=Window(float(session.group(1)) if session else None,
                       _parse_reset(session.group(2), now) if session and session.group(2) else None),
        weekly=Window(float(week.group(1)) if week else None,
                      _parse_reset(week.group(2), now) if week and week.group(2) else None),
        now=now,
    )


def _parse_reset(text: str, now: datetime) -> datetime | None:
    m = _RESET_RE.search(text)
    if not m:
        return None
    tz = ZoneInfo(m.group("tz"))
    month = datetime.strptime(m.group("month"), "%b").month
    hour = int(m.group("hour")) % 12
    if m.group("meridiem").lower() == "pm":
        hour += 12
    minute = int(m.group("minute") or 0)
    local_now = now.astimezone(tz)
    candidate = None
    for year in (local_now.year, local_now.year + 1):
        candidate = datetime(year, month, int(m.group("day")), hour, minute, tzinfo=tz)
        if candidate >= local_now:
            return candidate
    return candidate


class ClaudeProvider(Provider):
    key = "claude"
    title = "Claude"
    login_args = ("auth", "login")
    install_hint = "Install Claude Code (https://claude.ai/code), then sign in."

    def fetch(self) -> Usage:
        return fetch_usage()

    def find_cli(self) -> str | None:
        return which("claude")

    def local_stats(self, now_epoch: float) -> dict | None:
        return claude_logs.stats(now_epoch)

    def agent_state(self, now_epoch: float) -> dict:
        return claude_logs.agent_state(now_epoch)
