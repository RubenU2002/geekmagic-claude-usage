"""Helpers shared by the tests of the application: a fake for every port (no real device, AI tool, drawing library,
desktop or operating system), readings to feed it, and a recorder of what it renders and sends to the screen.

The application only knows the ports of geekmagic.core.ports, so a test builds the controller with these fakes through the
same composition root the real app uses (geekmagic.bootstrap) and drives it directly: no tray icon, no desktop session.
"""

from __future__ import annotations

import logging
import tempfile
import unittest
from contextlib import contextmanager
from datetime import datetime, timedelta
from pathlib import Path

from geekmagic.adapters.store.json_store import JsonStateStore
from geekmagic.application import config
from geekmagic.application.tray_controller import TrayController
from geekmagic.bootstrap import build_tray_controller
from geekmagic.core.errors import UsageError
from geekmagic.core.model import Usage, Window
from geekmagic.core.ports import Provider, Rendered
from geekmagic.core.registry import ProviderRegistry


def usage(current=10.0, weekly=5.0, title="Claude", *, current_reset=None, weekly_reset=None, **extra) -> Usage:
    """A provider's reading, as the screens and the app take it. `extra` sets any other field of Usage
    (working, waiting, stale, activity, codex_extra, pace...)."""
    now = datetime.now().astimezone()
    return Usage(
        title=title, now=now,
        current=Window(current, current_reset or now + timedelta(hours=2)),
        weekly=Window(weekly, weekly_reset or now + timedelta(days=3)), **extra)


def failing(error: Exception):
    """A reader that raises `error` (for fake_fetchers)."""
    def read():
        raise error
    return read


IDLE = {"working": False, "waiting": False, "waiting_for": None, "since": None, "last": None, "project": None, "sessions": []}
_readers: dict[str, object] = {}  # provider key -> what its fake reads (set by fake_fetchers)


@contextmanager
def fake_fetchers(**readers):
    """What each fake provider reads: `fake_fetchers(claude=lambda: usage(...), codex=failing(UsageError("...")))`."""
    saved = dict(_readers)
    _readers.update(readers)
    try:
        yield
    finally:
        _readers.clear()
        _readers.update(saved)


class FakeProvider(Provider):
    """A provider that reads what the test says. Its other methods are plain attributes a test can replace."""

    login_args = ("login",)

    def __init__(self, key: str, title: str) -> None:
        self.key, self.title = key, title
        self.install_hint = f"Install the {title} CLI."
        self.cli = f"/bin/{key}"
        self.stats = lambda now_epoch: {}
        self.agent = lambda now_epoch: dict(IDLE)

    def fetch(self) -> Usage:
        reader = _readers.get(self.key)
        if reader is None:
            raise UsageError(f"{self.title}: nothing to read in this test")
        return reader()

    def find_cli(self) -> str | None:
        return self.cli

    def local_stats(self, now_epoch: float) -> dict | None:
        return self.stats(now_epoch)

    def agent_state(self, now_epoch: float) -> dict:
        return self.agent(now_epoch)


class Capture:
    """What the app rendered and sent to the screen while a `capture_screen` block was open."""

    def __init__(self) -> None:
        self.single: list[tuple[Usage, str]] = []  # (reading, animation asked for) of each single-provider screen
        self.errors: list = []  # each "can't read it" screen (an ErrorScreen)
        self.panels: list[tuple[str, list[Usage], str]] = []  # (view, the readings, animation asked for) of each view of two
        self.uploads: list[str] = []  # file names uploaded to the device
        self.shown: list[str] = []  # file names pinned on its screen


_capture: Capture | None = None


@contextmanager
def capture_screen():
    """Record what would have been drawn and uploaded (the fakes always accept everything; this is how a test looks)."""
    global _capture
    previous, _capture = _capture, Capture()
    try:
        yield _capture
    finally:
        _capture = previous


class FakeRenderer:
    """The Renderer port: draws nothing, records what it was asked."""

    version = "test"

    def __init__(self) -> None:
        self.confirmed: list[Rendered] = []
        self.saved: dict = {}

    def render(self, key, data, animation) -> Rendered:
        from geekmagic.core.model import ErrorScreen
        from geekmagic.core.views import PANEL_KEYS
        if _capture is not None:
            if key in PANEL_KEYS:
                _capture.panels.append((key, data, animation))
            elif isinstance(data, ErrorScreen):
                _capture.errors.append(data)
            else:
                _capture.single.append((data, animation))
        return Rendered(b"GIF89a", [animation])

    def confirm(self, rendered) -> None:
        self.confirmed.append(rendered)

    def restore(self, saved: dict) -> None:
        self.saved = saved

    def snapshot(self) -> dict:
        return {}


class FakeDisplay:
    """The Display port: a screen that answers everything. Replace a method on the instance to make it fail."""

    def __init__(self, ip: str) -> None:
        self.ip = ip
        self.files: set[str] = set()  # what list_images says is stored
        self.calls: list[tuple] = []

    def upload(self, image_bytes, filename, content_type="image/gif", cancel=None) -> None:
        self.calls.append(("upload", filename))
        if _capture is not None:
            _capture.uploads.append(filename)

    def list_images(self) -> set[str]:
        return set(self.files)

    def delete_image(self, filename) -> None:
        self.calls.append(("delete", filename))

    def show_image(self, filename) -> None:
        self.calls.append(("show", filename))
        if _capture is not None:
            _capture.shown.append(filename)

    def set_brightness(self, level) -> None:
        self.calls.append(("brightness", level))

    def set_night_mode(self, *, start_hour, end_hour, night_level, day_level, enabled) -> None:
        self.calls.append(("night", start_hour, end_hour, night_level, day_level, enabled))


class FakeLocator:
    """The DeviceLocator port: a screen at `found` (or nowhere)."""

    def __init__(self) -> None:
        self.reachable = True  # does probe() find it
        self.found: str | None = None  # what find_device() returns
        self.probes: list[str] = []
        self.searches: list = []
        self.local: list[str] = ["192.168.1."]  # the networks the computer is on

    def probe(self, host, timeout=2.0) -> bool:
        self.probes.append(host)
        return self.reachable

    def find_device(self, prefer=None):
        self.searches.append(prefer)
        return self.found

    def networks(self) -> list[str]:
        return list(self.local)


class FakeTerminal:
    """The TerminalLauncher port: records what it was asked to open."""

    def __init__(self) -> None:
        self.opened: list[tuple[list[str], str]] = []
        self.outcome = "started"

    def launch(self, argv, title) -> str:
        self.opened.append((list(argv), title))
        return self.outcome


class FakeLockSensor:
    def __init__(self) -> None:
        self.locked = False

    def is_locked(self) -> bool:
        return self.locked


class FakeNotifier:
    """The system's own notification route (the backup)."""

    def __init__(self) -> None:
        self.sent: list[tuple[str, str]] = []

    def send(self, title, message) -> None:
        self.sent.append((title, message))


class FakeUi:
    """The Ui port: records what the application tells the tray icon. Its own notification is the primary one."""

    def __init__(self, notes: list | None = None) -> None:
        self.titles: list[str] = []
        self.icons: list[str] = []
        self.menu_refreshes = 0
        self.notes: list[tuple[str, str]] = notes if notes is not None else []  # notifications shown: (title, message)
        self.fail_notifications = False
        self.running = False

    @property
    def title(self) -> str | None:
        return self.titles[-1] if self.titles else None

    def set_title(self, title: str) -> None:
        self.titles.append(title)

    def set_icon(self, key: str) -> None:
        self.icons.append(key)

    def refresh_menu(self) -> None:
        self.menu_refreshes += 1

    def send(self, title: str, message: str) -> None:
        if self.fail_notifications:
            raise RuntimeError("no toast support")
        self.notes.append((title, message))

    def run(self) -> None:
        self.running = True

    def stop(self) -> None:
        self.running = False


class AppTestCase(unittest.TestCase):
    """A tray controller on fakes. After `app = self.make()`: `app.ui` records what the icon was told (`self.notes` are
    its notifications, `self.backups` those the system route had to send instead); the screen is a FakeDisplay."""

    def setUp(self) -> None:
        logging.disable(logging.CRITICAL)  # the tests provoke errors on purpose; keep their log lines off the console
        self.addCleanup(logging.disable, logging.NOTSET)
        self.tmp = Path(tempfile.mkdtemp())
        self.store = JsonStateStore(self.tmp / "state.json")  # the real file store, in a temp folder
        self.fake_notifier = FakeNotifier()
        self._notes: list[tuple[str, str]] = []  # what every icon of this test was asked to notify
        self.locator = FakeLocator()
        self.terminal = FakeTerminal()
        self.lock_sensor = FakeLockSensor()
        self.displays: list[FakeDisplay] = []
        self._saved_config = config.BACKGROUND_STATS
        config.BACKGROUND_STATS = False  # count the activity inline so tests are deterministic
        self.addCleanup(setattr, config, "BACKGROUND_STATS", self._saved_config)
        self.addCleanup(_readers.clear)

    @property
    def notes(self) -> list[tuple[str, str]]:
        return self._notes

    @property
    def backups(self) -> list[tuple[str, str]]:
        return self.fake_notifier.sent

    def make(self, ip: str = "192.168.1.50", provider: str | None = None, *, night_hours=None, night_level=None,
             **ports) -> TrayController:
        """A controller on the fakes; `ports` replaces any of them (store, providers, renderer, ...)."""
        def display_factory(address: str) -> FakeDisplay:
            display = FakeDisplay(address)
            self.displays.append(display)
            return display

        self._ui = FakeUi(self._notes)
        wired = {
            "providers": ProviderRegistry([FakeProvider("claude", "Claude"), FakeProvider("codex", "Codex")]),
            "store": self.store, "display_factory": display_factory, "locator": self.locator, "renderer": FakeRenderer(),
            "notifier": self.fake_notifier, "lock_sensor": self.lock_sensor, "terminal": self.terminal, **ports,
        }
        app = build_tray_controller(self._ui, ip=ip, interval=30, animation="idle", provider=provider,
                                    night_hours=night_hours, night_level=night_level, **wired)
        app.ui = self._ui
        return app
