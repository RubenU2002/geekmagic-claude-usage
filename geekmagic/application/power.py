"""Pausing: by hand, and by itself while the computer is locked."""

from __future__ import annotations

import logging
import threading
from collections.abc import Callable

from geekmagic.application import config
from geekmagic.core.ports import LockSensor

log = logging.getLogger("tray")


class Power:
    """Whether the app should be querying and uploading. `on_change()` is called when that, or why, changes."""

    def __init__(self, sensor: LockSensor, on_change: Callable[[], None], on_lock_change: Callable[[bool], None],
                 wake: threading.Event, stop: threading.Event, save: Callable[[], None]) -> None:
        self.sensor = sensor
        self.on_change = on_change
        self.on_lock_change = on_lock_change  # (locked) -> None: dim the backlight, put the view back
        self.wake = wake
        self.stop = stop
        self.save = save
        self.paused = False  # manual pause (not remembered: a forgotten pause would be a nasty surprise)
        self.locked = False
        self.pause_on_lock = True  # stop while the computer is locked

    def restore(self, saved: dict) -> None:
        self.pause_on_lock = bool(saved.get("pause_on_lock", True))

    def snapshot(self) -> dict:
        return {"pause_on_lock": self.pause_on_lock}

    def is_paused(self) -> bool:
        return self.paused or (self.pause_on_lock and self.locked)

    def toggle_pause(self) -> None:
        self.paused = not self.paused
        log.info("paused" if self.paused else "resumed")
        self.on_change()
        if not self.paused:
            self.wake.set()  # catch up at once

    def toggle_pause_on_lock(self) -> None:
        self.pause_on_lock = not self.pause_on_lock
        log.info("pause when locked %s", "on" if self.pause_on_lock else "off")
        self.save()
        self.on_change()
        self.wake.set()

    def watch_lock(self) -> None:
        """Thread: notice the computer being locked or unlocked, within a few seconds."""
        while not self.stop.is_set():
            locked = self.sensor.is_locked()
            if locked != self.locked:
                self.set_locked(locked)
            self.stop.wait(config.LOCK_POLL)

    def set_locked(self, locked: bool) -> None:
        self.locked = locked
        log.info("computer %s", "locked" if locked else "unlocked")
        self.on_change()
        self.on_lock_change(locked)
        if not locked:
            self.wake.set()  # refresh at once and put the view back
