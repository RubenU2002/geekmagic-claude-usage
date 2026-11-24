"""Reading each provider's usage, and what is made of it: the recent pace, the alerts, which readings have gone stale."""

from __future__ import annotations

import logging
import time
from collections.abc import Callable

from geekmagic.application import config
from geekmagic.core.errors import SignInNeeded, UsageError
from geekmagic.core import alerts, pace
from geekmagic.core.model import Usage
from geekmagic.core.registry import ProviderRegistry

log = logging.getLogger("tray")


class UsageHistory:
    """The recent readings of each usage window, from which the current pace is measured."""

    def __init__(self) -> None:
        self.points: dict[str, list] = {}  # "provider/window" -> [[epoch, pct, reset epoch], ...]

    def record(self, provider: str, usage: Usage) -> None:
        """Keep the readings of each window (when the percentage changed, or every few minutes) so the recent pace can
        be measured. Readings of an earlier window are dropped as soon as it resets."""
        now = time.time()
        for window, _ in config.WINDOWS:
            pct, reset = usage.window(window).pct, usage.window(window).reset
            if pct is None or reset is None:
                continue
            points = self.points.setdefault(f"{provider}/{window}", [])
            points[:] = [p for p in points if abs(p[2] - reset.timestamp()) < 120]
            if not points or points[-1][1] != pct or now - points[-1][0] >= config.HISTORY_EVERY[window]:
                points.append([now, pct, reset.timestamp()])
            del points[:-config.HISTORY_MAX]

    def rates(self, provider: str, usage: Usage) -> dict[str, float | None]:
        rates = {}
        for window, _ in config.WINDOWS:
            reset = usage.window(window).reset
            rates[window] = pace.recent_rate(
                self.points.get(f"{provider}/{window}", []), reset.timestamp() if reset else None,
                time.time(), pace.LOOKBACK_MIN[window],
            )
        return rates

    def restore(self, saved: dict) -> None:
        self.points = {k: [list(p) for p in v if isinstance(p, (list, tuple)) and len(p) == 3][-config.HISTORY_MAX:]
                       for k, v in saved.get("history", {}).items() if isinstance(v, list)}

    def snapshot(self) -> dict:
        return {"history": self.points}


class AlertTracker:
    """Which usage alerts already fired, so each fires once (also across restarts)."""

    def __init__(self, providers: ProviderRegistry, notify: Callable[[str, str], None], save: Callable[[], None]) -> None:
        self.providers = providers
        self.notify = notify
        self.save = save
        self.state: dict[str, dict] = {}  # "provider/window" -> {"level": 0..2, "pct": last reading}

    def check(self, provider: str, usage: Usage) -> None:
        """Notify when a window newly reaches 80 % / 95 %, and when it resets after that."""
        for window, label in config.WINDOWS:
            pct = usage.window(window).pct
            if pct is None:
                continue
            key = f"{provider}/{window}"
            previous = self.state.get(key, {})
            level, event = alerts.evaluate(previous.get("level", 0), previous.get("pct"), pct)
            self.state[key] = {"level": level, "pct": pct}
            if event is None:
                continue
            kind, threshold = event
            if kind == "rise":
                self.notify(self.providers.titles[provider], f"La {label} llegó al {pct:.0f} % (umbral {threshold} %)")
            else:
                self.notify(self.providers.titles[provider], f"La {label} se reinició ({pct:.0f} % usado)")
        self.save()

    def restore(self, saved: dict) -> None:
        self.state = {k: v for k, v in saved.get("alerts", {}).items() if isinstance(v, dict)}

    def snapshot(self) -> dict:
        return {"alerts": self.state}


class UsageService:
    """Reads the providers and keeps the latest good reading of each, with why the last read failed if it did.

    The app hooks in through `on_read(provider)` (after a good read) and `on_failure(provider, error)` (after a bad one).
    """

    def __init__(self, providers: ProviderRegistry, history: UsageHistory, alert_tracker: AlertTracker, lock) -> None:
        self.providers = providers
        self.history = history
        self.alert_tracker = alert_tracker
        self.lock = lock
        self.last_fetch: dict[str, float] = {}
        self.failures: dict[str, int] = {}  # provider -> consecutive failed reads
        self.retry_at: dict[str, float] = {}  # provider -> not before this (monotonic) once it keeps failing
        self.last_error: dict[str, Exception] = {}
        self.last_good: dict[str, tuple[Usage, float]] = {}  # provider -> (last usage that was read OK, when)
        self.stale: set[str] = set()  # providers whose image on the device is the dimmed "no fresh data" one
        self.errors: dict[str, str] = {}  # provider -> why its last read failed (gone once a read works)
        self.needs_login: set[str] = set()  # providers whose last read failed because you're signed out
        self.on_read: Callable[[str], None] = lambda provider: None
        self.on_failure: Callable[[str, Exception], None] = lambda provider, error: None

    def fetch(self, provider: str) -> Usage | None:
        """Query the provider. Every successful read also feeds the alerts; a failing one may mark the data stale.
        A provider that keeps failing (no internet, a VPN...) is left alone for a while instead of making every cycle
        wait for its timeout: see `reset_backoff`."""
        now = time.monotonic()
        if now < self.retry_at.get(provider, 0):
            self.on_failure(provider, self.last_error[provider])  # (still failing: it keeps the stale dimming going)
            return None
        self.last_fetch[provider] = now
        try:
            usage = self.providers[provider].fetch()
        except Exception as e:  # keep the tray alive no matter what
            if isinstance(e, UsageError):  # an expected problem (not signed in...): its message says it all
                log.warning("fetch failed (%s): %s", provider, e)
            else:
                log.exception("fetch failed (%s)", provider)
            if isinstance(e, SignInNeeded):
                self.needs_login.add(provider)
            self.errors[provider] = str(e)
            self._back_off(provider, e)
            self.on_failure(provider, e)
            return None
        with self.lock:  # (the slow part, the read itself, was above and ran unlocked)
            self.history.record(provider, usage)
            pace.annotate(usage, self.history.rates(provider, usage))  # the pace that goes under each bar
            self.last_good[provider] = (usage, time.monotonic())
            self.stale.discard(provider)
            self.errors.pop(provider, None)
            self.needs_login.discard(provider)
            self.failures.pop(provider, None)
            self.retry_at.pop(provider, None)
            self.last_error.pop(provider, None)
            self.on_read(provider)
            self.alert_tracker.check(provider, usage)  # (which also saves)
        return usage

    def _back_off(self, provider: str, error: Exception) -> None:
        self.last_error[provider] = error
        count = self.failures[provider] = self.failures.get(provider, 0) + 1
        if count >= config.BACKOFF_FROM:
            wait = min(config.BACKOFF_BASE * 2 ** (count - config.BACKOFF_FROM), config.BACKOFF_MAX)
            self.retry_at[provider] = time.monotonic() + wait
            log.info("%s failed %d times in a row; not reading it again for %d s", provider, count, wait)

    def reset_backoff(self) -> None:
        """Try every provider at the next cycle (you asked for it: a click, "Actualizar ahora")."""
        self.retry_at.clear()

    def recently_read(self, provider: str, within: float) -> bool:
        good = self.last_good.get(provider)
        return good is not None and time.monotonic() - good[1] < within

    def stale_candidate(self, provider: str) -> Usage | None:
        """The last good reading, if it has been too long since it was refreshed and it isn't already shown as stale."""
        good = self.last_good.get(provider)
        if not good or provider in self.stale:
            return None
        usage, read_at = good
        if time.monotonic() - read_at < config.STALE_SECONDS:
            return None
        return usage

    def signed_out(self, provider: str) -> bool:
        """Did its last read fail because you're signed out? (Only a read can tell: each CLI says so in its own way.)"""
        return provider in self.needs_login
