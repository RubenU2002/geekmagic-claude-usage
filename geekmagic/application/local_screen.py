"""The pictures of the screen, kept on the computer too: what the simulated screen shows when there is no GeekMagic to show
them (it is switched off, on another network, not found yet...).

Every picture drawn for the device is remembered here, and while the device can't be reached the pictures are still drawn
(see Screen.show_locally), so the tray icon can be the screen: a click opens a window that plays the active view's GIF.
"""

from __future__ import annotations

import threading
from collections.abc import Callable


class LocalScreen:
    def __init__(self, active_key: Callable[[], str]) -> None:
        self.active_key = active_key
        self._images: dict[str, tuple[int, bytes]] = {}  # view -> (version, its GIF)
        self._drawn_from: dict[str, object] = {}  # view -> what that picture was drawn from
        self._version = 0
        self._lock = threading.Lock()

    def remember(self, key: str, gif: bytes, drawn_from: object = None) -> None:
        """`gif` is now the picture of the view `key`; `drawn_from` is what it was drawn from (see is_current)."""
        with self._lock:
            self._version += 1
            self._images[key] = (self._version, gif)
            self._drawn_from[key] = drawn_from

    def is_current(self, key: str, drawn_from: object) -> bool:
        """Is the picture of `key` already drawn from exactly this? (Then there is no point in drawing it again.)"""
        with self._lock:
            return key in self._images and key in self._drawn_from and self._drawn_from[key] == drawn_from

    def latest(self) -> tuple[str, int, bytes] | None:
        """(view, version, GIF) of the view that should be on screen, None if it has no picture yet. The version changes
        whenever that picture does, so a window can tell without comparing bytes."""
        key = self.active_key()
        with self._lock:
            found = self._images.get(key)
        return (key, found[0], found[1]) if found else None
