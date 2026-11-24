"""The ports: what the application needs from the outside world, as interfaces.

The application (geekmagic.application) is written against these and nothing else, so it knows no AI tool, no screen
hardware, no drawing library and no operating system. Each adapter (geekmagic.adapters) implements one of them; the
composition root (geekmagic.bootstrap) is the only place that picks which implementation to use.

Driven ports, implemented by adapters the application calls:
    Provider       an AI tool: read its usage, count its logs, see whether its agent is busy, sign in
    Display        the GeekMagic screen: store an image, show it, set the backlight
    DeviceLocator  finding that screen on the network
    Renderer       drawing the screens (readings in, a GIF out)
    Notifier       showing a desktop notification
    StateStore     remembering things between runs
    LockSensor     is the computer locked
    TerminalLauncher  open a terminal window that runs a command (a provider's sign-in)

Driving port, implemented by the adapter that calls the application:
    Ui             the tray icon: its title, its icon, its menu, its own notification
"""

from __future__ import annotations

import threading
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Protocol

from geekmagic.core.model import ErrorScreen, Usage


class Provider(ABC):
    """An AI tool (Claude, Codex...). Everything that is specific to one lives behind this interface, so supporting
    another is one new class and an entry in the composition root."""

    key: str  # "claude": what is stored on disk and used as a file name
    title: str  # "Claude": what the screens and notifications call it
    login_args: tuple[str, ...]  # the arguments of its CLI that start the sign-in
    install_hint: str  # what to tell someone whose CLI isn't installed

    @abstractmethod
    def fetch(self) -> Usage:
        """Its current usage. Raises UsageError, or SignInNeeded when signing in again is what's needed. Never spends
        a token."""

    @abstractmethod
    def find_cli(self) -> str | None:
        """Where its command-line tool is installed, or None."""

    @abstractmethod
    def local_stats(self, now_epoch: float) -> dict | None:
        """Its activity counted from its local logs (see geekmagic.core.activity), None without logs."""

    @abstractmethod
    def agent_state(self, now_epoch: float) -> dict:
        """Whether its agent is working or waiting for you right now: {"working", "waiting", "waiting_for", "since",
        "last", "project", "sessions": [...]}."""

    def __repr__(self) -> str:
        return f"<{type(self).__name__} {self.key}>"


class Display(Protocol):
    """One GeekMagic screen. Every method raises OSError when it doesn't answer properly; `upload` raises
    geekmagic.core.errors.UploadCancelled when `cancel` is set mid-transfer."""

    ip: str

    def upload(self, image_bytes: bytes, filename: str, content_type: str = "image/gif",
               cancel: threading.Event | None = None) -> None: ...

    def list_images(self) -> set[str]: ...

    def delete_image(self, filename: str) -> None: ...

    def download_image(self, filename: str) -> bytes:
        """The stored image file, as it is on the device (what the screen is showing, when it is pinned)."""

    def show_image(self, filename: str) -> None: ...

    def set_brightness(self, level: float) -> None: ...

    def set_night_mode(self, *, start_hour: int, end_hour: int, night_level: float, day_level: float,
                       enabled: bool) -> None: ...


class DeviceLocator(Protocol):
    def probe(self, host: str, timeout: float = 2.0) -> bool:
        """Does a GeekMagic screen answer at this address?"""

    def find_device(self, prefer: str | None = None) -> str | None:
        """Look for it on the local network (`prefer` is tried first)."""

    def networks(self) -> list[str]:
        """The networks this computer is on, as "192.168.1." prefixes, the main one first ([] without any)."""


@dataclass
class Rendered:
    """What a Renderer returns: the GIF to upload, and which animations it picked from its rotation, to be remembered
    (Renderer.confirm) only once the upload has gone through."""
    gif: bytes
    animations: list[str] = field(default_factory=list)  # what each panel played, in order (for the log)
    picks: list[tuple[str, str]] = field(default_factory=list)  # (provider title, animation)


class Renderer(Protocol):
    version: str  # identifies the drawing code: a stored image drawn by another version is redrawn

    def render(self, key: str, data: Usage | list[Usage] | ErrorScreen, animation: str) -> Rendered:
        """The screen for `key`: a provider's key (`data` is its Usage, or an ErrorScreen), or one of the views of two
        (core.views; `data` is the list of the two Usage). `animation` is "auto"/"random" or the name of one."""

    def to_png(self, gif: bytes) -> bytes:
        """A still picture (PNG) of a GIF this renderer drew."""

    def confirm(self, rendered: Rendered) -> None:
        """The upload went through: remember what was picked, so it doesn't come back too soon."""

    def restore(self, saved: dict) -> None: ...

    def snapshot(self) -> dict: ...


class Notifier(Protocol):
    def send(self, title: str, message: str) -> None:
        """Show a notification. Raises if it couldn't."""


class StateStore(Protocol):
    def load(self) -> dict:
        """What was saved, or {} if there's nothing (or nothing readable)."""

    def save(self, state: dict) -> None: ...


class LockSensor(Protocol):
    def is_locked(self) -> bool: ...


class TerminalLauncher(Protocol):
    def launch(self, argv: list[str], title: str) -> str:
        """Run `argv` in a new terminal window that stays open: "started", or "failed"."""


class Ui(Protocol):
    """The tray icon, from the application's side. It is also a Notifier: its own notification is the one shown."""

    def set_title(self, title: str) -> None: ...

    def set_icon(self, key: str) -> None:
        """Show the icon of a view (a provider's key, or one of the views of two)."""

    def refresh_menu(self) -> None: ...

    def send(self, title: str, message: str) -> None: ...

    def ask_save_path(self, suggested_name: str) -> str | None:
        """Ask where to save a picture (a .png, or a .gif to keep it animated): the path chosen, None if cancelled."""

    def show_simulator(self) -> None:
        """Open the window that simulates the screen (or bring it to the front if it is open)."""

    def toggle_simulator(self) -> None:
        """Open the simulated screen's window, or close it if it is open."""

    def run(self) -> None:
        """Show the icon and handle its clicks; returns when it is stopped."""

    def stop(self) -> None: ...
