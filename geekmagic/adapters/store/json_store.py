"""What the app remembers between runs, in one JSON file.

Each part of the app that has something to remember implements `restore(saved)` and `snapshot()`; this only reads and
writes the file, so a damaged one costs nothing but the memory.
"""

from __future__ import annotations

import json
import logging
from pathlib import Path

log = logging.getLogger("tray")


class JsonStateStore:
    """The StateStore port: one JSON file."""

    def __init__(self, path: Path) -> None:
        self.path = path

    def load(self) -> dict:
        """What was saved, or {} if there's nothing (or nothing readable)."""
        try:
            saved = json.loads(self.path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return {}
        return saved if isinstance(saved, dict) else {}

    def save(self, state: dict) -> None:
        try:
            self.path.write_text(json.dumps(state), encoding="utf-8")
        except OSError:
            log.warning("could not save %s", self.path)
