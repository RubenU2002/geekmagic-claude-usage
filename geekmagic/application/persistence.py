"""Saving and restoring the parts of the app that have something to remember, as one file."""

from __future__ import annotations

import logging
import threading

from geekmagic.core.ports import StateStore

log = logging.getLogger("tray")


class Persistence:
    def __init__(self, store: StateStore) -> None:
        self.store = store
        self.parts: list = []  # each has restore(saved) and snapshot() -> dict
        # Guards the readings, history, alerts and the state file: the providers are read in parallel threads.
        self.lock = threading.RLock()

    def register(self, *parts) -> None:
        self.parts.extend(parts)

    def load(self) -> dict:
        """Hand what was saved to every part. A part that can't make sense of its piece starts afresh; the others
        aren't affected."""
        saved = self.store.load()
        for part in self.parts:
            try:
                part.restore(saved)
            except Exception:
                log.warning("could not restore %s from %s", type(part).__name__, self.store.path, exc_info=True)
        return saved

    def save(self) -> None:
        with self.lock:
            state: dict = {}
            for part in self.parts:
                state.update(part.snapshot())
            self.store.save(state)
