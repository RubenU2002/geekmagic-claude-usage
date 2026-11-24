"""The providers' activity, counted from their local logs (up to a month of them) for the stats views."""

from __future__ import annotations

import logging
import threading
import time
from collections.abc import Callable

from geekmagic.application import config
from geekmagic.core.registry import ProviderRegistry

log = logging.getLogger("tray")


class ActivityCounter:
    """Counts now and then, not every cycle, and in a thread, since the first count takes seconds. Until it's done the
    screens say "Counting...". `on_done()` wakes the loop to redraw the screen that was saying so."""

    def __init__(self, providers: ProviderRegistry, on_done: Callable[[], None]) -> None:
        self.providers = providers
        self.on_done = on_done
        # provider -> its activity from its local logs (see insights.usage_stats); {} = it has none, missing = not counted yet
        self.stats: dict[str, dict] = {}
        self.counted_at = 0.0
        self.counting = False  # the counts are being computed in a thread

    def refresh(self) -> None:
        since = time.monotonic() - self.counted_at
        complete = len(self.stats) == len(self.providers)
        if self.counting or since < (config.LOCAL_STATS_EVERY if complete else config.LOCAL_STATS_RETRY):
            return
        self.counted_at = time.monotonic()
        if config.BACKGROUND_STATS:
            self.counting = True
            threading.Thread(target=self._count, daemon=True).start()
        else:
            self._count()

    def _count(self) -> None:
        try:
            for provider in self.providers.values():
                try:
                    stats = provider.local_stats(time.time())
                    self.stats[provider.key] = stats if stats is not None else {}
                except Exception:
                    log.warning("could not count %s's local activity", provider.key, exc_info=True)
        finally:
            self.counting = False
            if config.BACKGROUND_STATS:
                self.on_done()  # redraw the screen that was saying "Counting..."
