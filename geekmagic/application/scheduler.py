"""The worker loop: read the usage, put it on the screen, and keep the screens that aren't showing fresh.

One cycle reads what's on screen (or both providers, for a view of two), uploads it, then refreshes at most one other
thing, so the device (which serves one request at a time) is left free for the next click.
"""

from __future__ import annotations

import logging
import threading
import time
from collections.abc import Callable
from dataclasses import replace
from datetime import datetime

from geekmagic.application import config
from geekmagic.application.activity import ActivityCounter
from geekmagic.application.agents import AgentMonitor
from geekmagic.application.power import Power
from geekmagic.application.screen import Screen
from geekmagic.application.signin import SignInCoordinator
from geekmagic.application.usage import UsageService
from geekmagic.application.viewstate import ViewState
from geekmagic.core.model import Usage, Window
from geekmagic.core.registry import ProviderRegistry
from geekmagic.core.views import PANEL_KEYS, SPLIT, STATS_VIEWS

log = logging.getLogger("tray")


class Scheduler:
    def __init__(self, providers: ProviderRegistry, interval: int, usage: UsageService, screen: Screen, view: ViewState, power: Power,
                 agents: AgentMonitor, activity: ActivityCounter, signin: SignInCoordinator,
                 wake: threading.Event, stop: threading.Event, tooltip: Callable[[], str],
                 set_title: Callable[[str], None]) -> None:
        self.providers = providers
        self.interval = interval
        self.usage = usage
        self.screen = screen
        self.view = view
        self.power = power
        self.agents = agents
        self.activity = activity
        self.signin = signin
        self.wake = wake
        self.stop = stop
        self.tooltip = tooltip
        self.set_title = set_title

    # --- reading what's needed for a view ---------------------------------------------------------------------------

    def fetch_all(self) -> None:
        """Read both providers at the same time (Claude's `/usage` takes ~3.5 s, Codex's ~1 s): a view of both waits
        for the slower one instead of for the two added up."""
        threads = [threading.Thread(target=self.usage.fetch, args=(provider,), daemon=True) for provider in self.providers]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()

    def panels(self, key: str) -> list[Usage]:
        """The two providers' latest readings for a view of both. One whose read failed keeps its last good numbers,
        dimmed and dated once they're a few minutes old; one that has never been read keeps its panel, with dashes."""
        panels = []
        anything = any(p in self.usage.last_good for p in self.providers)  # (two panels of dashes would say nothing at all)
        for provider in self.providers:
            good = self.usage.last_good.get(provider)
            if good is None:
                if provider in self.usage.errors and anything:
                    usage = Usage(title=self.providers.titles[provider], current=Window(), weekly=Window(), now=datetime.now().astimezone())
                    if key in STATS_VIEWS:
                        usage.activity = self.activity.stats.get(provider)
                    panels.append(usage)
                continue
            usage, read_at = good
            usage = replace(usage, stale=True) if time.monotonic() - read_at >= config.STALE_SECONDS else replace(usage)
            if key in STATS_VIEWS:
                usage.activity = self.activity.stats.get(provider)
            panels.append(usage)
        return panels

    def update_panels(self, key: str, refetch: bool = True) -> str:
        """A view of both providers: read both (so both get alerts) and upload one screen with the two of them.
        `refetch=False` redraws from the readings we already have (only an agent started or stopped)."""
        if refetch:
            self.fetch_all()
        if key in STATS_VIEWS:
            self.activity.refresh()
        panels = self.panels(key)
        if not panels:
            return "error"
        return self.screen.deliver(key, panels)

    def update_split(self) -> str:
        return self.update_panels(SPLIT)

    def recently_read(self, provider: str) -> bool:
        return self.usage.recently_read(provider, self.interval)

    # --- keeping the other screens fresh ----------------------------------------------------------------------------

    def mark_stale(self, provider: str) -> None:
        """Numbers that can't be refreshed for a few minutes: put the last ones on screen, dimmed and dated."""
        s = self.screen
        if not s.ip or s.offline or self.view.mode:
            return  # (the views of two dim stale panels themselves, see panels)
        usage = self.usage.stale_candidate(provider)
        if usage is None:
            return
        self.usage.stale.add(provider)
        log.warning("no fresh %s data for %d s; showing the last reading dimmed", provider, config.STALE_SECONDS)
        try:
            s.upload(provider, replace(usage, stale=True))
        except Exception:
            log.exception("could not upload the stale image (%s)", provider)

    def refresh_other(self, provider: str | None = None) -> None:
        """The provider that isn't on screen: read it now and then (its alerts matter too) and keep its image fresh.
        One per cycle."""
        s = self.screen
        for other in (p for p in self.providers if p != provider):
            # nothing (or only an out-of-date image) of it on the device yet: do it now, so the first click is instant too
            needs_image = (other not in s.slots or other in s.outdated) and not s.offline
            if not needs_image and time.monotonic() - self.usage.last_fetch.get(other, 0) < config.OTHER_REFRESH:
                continue
            usage = self.usage.fetch(other) or self.signin.error_usage(other)
            if usage is not None and s.ip and not s.offline:
                try:
                    s.upload(other, usage)
                except Exception:
                    log.warning("could not upload the %s image", other, exc_info=True)
            return

    def refresh_outdated_views(self) -> None:
        """After an update the images stored on the device were drawn by the old code (old labels, colours...), and a
        view is only redrawn while it's on screen. Redraw the ones not on screen, from the latest readings, without
        showing them. One per cycle, so the device stays free for clicks."""
        s = self.screen
        if s.offline or not s.ip:
            return
        for key in sorted(s.outdated - {self.view.key}):
            if key in PANEL_KEYS:
                if key in STATS_VIEWS:
                    self.activity.refresh()
                usage = self.panels(key)
                ready = len(usage) == len(self.providers)  # (half a view would be stored as if it were whole)
            else:
                good = self.usage.last_good.get(key)
                usage, ready = (good[0] if good else None), good is not None
            if not ready:
                continue  # nothing read for it yet; a later cycle will have something
            try:
                s.upload(key, usage)
            except Exception:
                log.warning("could not refresh the stored %s image", key, exc_info=True)
            return

    # --- the loop -------------------------------------------------------------------------------------------------

    def run(self) -> None:
        s = self.screen
        pending: tuple[str, dict, float] | None = None  # (provider, usage, read at) awaiting delivery
        if s.ip:  # the remembered view goes up first, in one request: it needs neither the search nor the file list
            s.show_uploaded(self.view.key)
        s.resolve_device()
        s.sync_with_device()
        if not s.slots.get(self.view.key) or s.shown_failed:
            s.show_uploaded(self.view.key)  # (the address changed, or the first try didn't get through)
        while not self.stop.is_set():
            self.wake.clear()
            # An upload keeps the device busy for seconds; wait out rapid toggling so a click never queues behind it.
            while not self.stop.is_set() and time.monotonic() - s.last_click < config.SETTLE_SECONDS:
                time.sleep(0.5)
            if self.power.is_paused():  # nothing is read or uploaded; coming back (resume / unlock) wakes the loop
                self.set_title(self.tooltip())
                self.wake.wait(config.PAUSE_POLL)
                continue
            provider = self.view.provider
            wait = self.interval
            if any(self.signin.pending(p) for p in (self.providers if self.view.mode else [provider])):
                wait = config.LOGIN_POLL  # someone is signing in right now: notice it as soon as it works
            # An agent started or stopped (and nothing else changed): redraw from the readings we already have.
            redraw_only, self.agents.dirty = self.agents.dirty, False
            if self.view.mode:
                recent = all(self.recently_read(p) for p in self.providers)
                if self.update_panels(self.view.mode, refetch=not (redraw_only and recent)) == "offline":
                    wait = config.RETRY_SECONDS
                    s.maybe_rediscover()
                self.refresh_outdated_views()
                self.wake.wait(wait)
                continue
            if pending and pending[0] == provider and time.monotonic() - pending[2] < self.interval:
                usage = pending[1]  # read moments ago but not delivered yet: don't query again
            elif redraw_only and self.recently_read(provider):
                usage = self.usage.last_good[provider][0]
            else:
                usage = self.usage.fetch(provider) or self.signin.error_usage(provider)
            pending = None
            if usage is not None:
                outcome = s.deliver(provider, usage)
                if outcome == "cancelled":  # retry with the same data once the clicking settles
                    pending = (provider, usage, time.monotonic())
                elif outcome == "offline":  # retry soon, without querying usage again
                    pending = (provider, usage, time.monotonic())
                    wait = config.RETRY_SECONDS
                    s.maybe_rediscover()
            self.refresh_other(provider)
            self.refresh_outdated_views()
            self.wake.wait(wait)
