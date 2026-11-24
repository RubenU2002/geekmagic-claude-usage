"""The composition root: the one place that knows which adapter implements each port.

Everything is built here and handed in, so the application never reaches for a concrete class. A test builds its own
controller with fakes in place of the adapters it doesn't want (see tests/support.py).
"""

from __future__ import annotations

from geekmagic import paths
from geekmagic.adapters.device.client import GeekMagicDevice
from geekmagic.adapters.device.discovery import Locator
from geekmagic.adapters.providers import default_providers
from geekmagic.adapters.render.renderer import PillowRenderer
from geekmagic.adapters.store.json_store import JsonStateStore
from geekmagic.adapters.system.notifier import SystemNotifier
from geekmagic.adapters.system.session_lock import SessionLockSensor
from geekmagic.adapters.system.terminal import Terminal
from geekmagic.application.tray_controller import TrayController
from geekmagic.core.registry import ProviderRegistry


def build_registry() -> ProviderRegistry:
    return ProviderRegistry(default_providers())


def build_tray_controller(ui, *, ip: str | None, interval: int, animation: str, provider: str | None = None,
                          night_hours: tuple[int, int] | None = None, night_level: int | None = None,
                          **ports) -> TrayController:
    """The tray app's controller on the real adapters; `ports` replaces any of them (providers, store, display_factory,
    locator, renderer, notifier, lock_sensor, terminal)."""
    defaults = {  # built only for the ports that weren't replaced
        "providers": build_registry, "store": lambda: JsonStateStore(paths.STATE_PATH), "display_factory": lambda: GeekMagicDevice,
        "locator": Locator, "renderer": PillowRenderer, "notifier": SystemNotifier, "lock_sensor": SessionLockSensor,
        "terminal": Terminal,
    }
    wired = {name: ports[name] if name in ports else make() for name, make in defaults.items()}
    unknown = set(ports) - set(defaults)
    assert not unknown, f"no such port: {unknown}"
    return TrayController(ui=ui, ip=ip, interval=interval, animation=animation, provider=provider,
                          night_hours=night_hours, night_level=night_level, **wired)


def run_tray(ip, interval, animation, provider, night_hours, night_level) -> None:
    """Show the tray icon and keep the screen up to date until it is quit."""
    from geekmagic.adapters.tray.ui import TrayUi  # (needs a desktop session: imported only when it's going to run)
    ui = TrayUi(paths.LOG_PATH)
    controller = build_tray_controller(ui, ip=ip, interval=interval, animation=animation, provider=provider,
                                       night_hours=night_hours, night_level=night_level)
    ui.attach(controller)
    controller.run()
