"""What the app passes around: a provider's usage, as typed objects.

Everything else (providers, the screens, the tray app) speaks these, so a typo in a field name is an error at once instead
of a screen that quietly shows dashes.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime

WINDOWS = ("current", "weekly")  # the two usage windows every provider has, in the order the screens show them


@dataclass
class Window:
    """One usage window: the 5-hour "session" (current) or the week."""
    pct: float | None = None  # percent used
    reset: datetime | None = None  # when it resets
    minutes: float | None = None  # how long the window is, when the provider says (Codex does); else a default is assumed


@dataclass
class Projection:
    """Where a window is heading at the pace you're going (see geekmagic.insights.pace)."""
    rate: float  # percentage points per minute
    end: float  # projected % at reset (may exceed 100)
    hits_limit: bool
    minutes_to_limit: float | None  # None if it won't run out


@dataclass
class CodexExtra:
    """The free rate-limit resets Codex has granted: how many are left, and in how many days the next one expires."""
    free_resets: int = 0
    next_reset_expires_days: int | None = None


@dataclass
class Usage:
    """A provider's usage right now, plus what the screens need to know about it."""
    title: str  # "Claude": who it belongs to (it also picks its mascot, colours and animations)
    current: Window
    weekly: Window
    now: datetime  # when it was read
    pace: dict[str, Projection | None] = field(default_factory=dict)  # per window ("current" / "weekly"), see insights.pace
    codex_extra: CodexExtra | None = None  # Codex only
    working: bool = False  # its agent is running right now
    waiting: bool = False  # ...and it has stopped and is waiting for you
    stale: bool = False  # these numbers couldn't be refreshed for a while
    activity: dict | None = None  # what the stats views show: None = still counting, {} = it has no logs

    def window(self, kind: str) -> Window:
        """The window called "current" or "weekly"."""
        if kind not in WINDOWS:
            raise KeyError(kind)
        return self.current if kind == "current" else self.weekly


@dataclass
class ErrorScreen:
    """What a provider's screen says when it has never been read."""
    title: str
    message: str
    now: datetime
    signin: bool = False  # signing in is what's needed (the heading says so)
