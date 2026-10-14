"""The tray app's controller: builds the use cases from the ports it is given, wires them together, and handles what the
icon's clicks and menu ask for.

It knows no desktop toolkit, no drawing library and no device: everything outside the application arrives through a port
(see geekmagic.core.ports), so it can be driven by anything that implements `Ui` (the tray icon, or a test).
"""

from __future__ import annotations

import logging
import threading
import time

from geekmagic.application import config
from geekmagic.application.activity import ActivityCounter
from geekmagic.application.agents import AgentMonitor
from geekmagic.application.backlight import Backlight
from geekmagic.application.notifications import Notifications
from geekmagic.application.persistence import Persistence
from geekmagic.application.power import Power
from geekmagic.application.scheduler import Scheduler
from geekmagic.application.screen import Screen
from geekmagic.application.signin import SignInCoordinator
from geekmagic.application.threads import in_background
from geekmagic.application.usage import AlertTracker, UsageHistory, UsageService
from geekmagic.application.viewstate import ViewState
from geekmagic.core.ports import (
    DeviceLocator, LockSensor, Notifier, Renderer, StateStore, TerminalLauncher, Ui,
)
from geekmagic.core.registry import ProviderRegistry
from geekmagic.core.views import VIEW_NAMES

log = logging.getLogger("tray")


class TrayController:
    def __init__(self, *, providers: ProviderRegistry, store: StateStore, display_factory, locator: DeviceLocator,
                 renderer: Renderer, notifier: Notifier, lock_sensor: LockSensor, terminal: TerminalLauncher, ui: Ui,
                 ip: str | None, interval: int, animation: str, provider: str | None = None,
                 night_hours: tuple[int, int] | None = None, night_level: int | None = None) -> None:
        """`display_factory(ip) -> Display`; `notifier` is the system's own route, the backup for `ui`'s notification."""
        self.providers = providers
        self.ui = ui
        self.wake = threading.Event()
        self.stop = threading.Event()
        self.renderer = renderer
        self.notifications = Notifications(backup=notifier, primary=ui)
        self.view = ViewState(providers, first_run_provider=provider)
        self.persistence = Persistence(store)
        save = self.persistence.save
        notify = self.notifications.send

        alert_tracker, history = AlertTracker(providers, notify, save), UsageHistory()
        self.usage = UsageService(providers, history, alert_tracker, self.persistence.lock)
        self.agents = AgentMonitor(providers, notify, self._agent_changed, self.stop,
                                   lambda: self.notifications.agent_events)
        self.activity = ActivityCounter(providers, self.wake.set)
        self.screen = Screen(
            ip, animation, providers, renderer, display_factory, locator,
            active_key=lambda: self.view.key, flag=self.agents.flag,
            notify=notify, save=save, set_title=ui.set_title, tooltip=self.tooltip, wake=self.wake,
        )
        self.backlight = Backlight(self.screen, notify, save, ui.refresh_menu, night_hours, night_level)
        self.power = Power(lock_sensor, self._refresh_title_and_menu, self._lock_changed, self.wake, self.stop, save)
        self.signin = SignInCoordinator(providers, self.usage, self.screen, notify, self.wake, terminal)
        self.usage.on_read = self.signin.on_read
        self.usage.on_failure = self._read_failed
        self.scheduler = Scheduler(
            providers, interval, self.usage, self.screen, self.view, self.power, self.agents, self.activity, self.signin,
            self.wake, self.stop, self.tooltip, ui.set_title,
        )

        self.persistence.register(
            self.view, self.notifications, renderer, alert_tracker, history, self.screen, self.backlight, self.power,
        )
        self.persistence.load()

    # --- what the icon says -----------------------------------------------------------------------------------------

    def tooltip(self) -> str:
        view = VIEW_NAMES[self.view.mode] if self.view.mode else f"mostrando {self.providers.titles[self.view.provider]}"
        notes = self.agents.tooltip_notes()
        if notes:
            view += " · " + " y ".join(notes)
        if self.power.paused:
            return f"GeekMagic: {view} (en pausa)"
        if self.power.pause_on_lock and self.power.locked:
            return f"GeekMagic: {view} (en pausa: PC bloqueado)"
        return f"GeekMagic: {view}"

    def _refresh_title_and_menu(self) -> None:
        self.ui.set_title(self.tooltip())
        self.ui.refresh_menu()

    # --- what the parts tell us -------------------------------------------------------------------------------------

    def _agent_changed(self) -> None:
        """An agent started or stopped: redraw with (or without) the working indicator, from the readings we have."""
        self.ui.set_title(self.tooltip())
        self.wake.set()

    def _read_failed(self, provider: str, error: Exception) -> None:
        self.signin.on_failure(provider, error)
        if provider == self.view.provider:
            self.ui.set_title(f"GeekMagic: {error}"[:120])
        self.scheduler.mark_stale(provider)

    def _lock_changed(self, locked: bool) -> None:
        self.backlight.on_lock_change(locked)
        if not locked:
            in_background(self.screen.show_uploaded, self.view.key)  # put the view back

    # --- clicks and menu commands -----------------------------------------------------------------------------------

    def _switched(self, key: str) -> None:
        """Common to every switch: pin the view's last image at once and refresh it in the background."""
        self.persistence.save()  # so a restart or reconnection comes back to this view
        self.usage.reset_backoff()  # you're looking at it: try again even if it has been failing
        self.ui.set_title(self.tooltip())
        self.ui.refresh_menu()
        self.screen.note_click()  # abort any upload in flight: the device serves one request at a time
        in_background(self.screen.show_uploaded, key)
        if time.monotonic() - self.screen.last_upload.get(key, 0) > config.STALE_AFTER:
            self.wake.set()  # refresh the image we just switched to, once clicking settles

    def select(self, provider: str) -> None:
        log.info("switch -> %s", provider)
        self.view.provider = provider
        self.view.mode = None
        self.ui.set_icon(provider)
        self._switched(provider)
        in_background(self.signin.ask, provider)  # (only does anything if it's signed out)

    def toggle(self) -> None:
        """Left-click: on to the next provider. From a view of both it goes back to the provider you were on."""
        if self.view.mode:
            self.select(self.view.provider)
            return
        keys = list(self.providers)
        self.select(keys[(keys.index(self.view.provider) + 1) % len(keys)])

    def toggle_mode(self, mode: str) -> None:
        """Menu: a view of both providers at once. Turning it off returns to the last single provider."""
        if self.view.mode == mode:
            self.select(self.view.provider)
            return
        log.info("switch -> %s view", mode)
        self.view.mode = mode
        self.ui.set_icon(mode)
        self._switched(mode)

    def select_action(self, provider: str):
        return lambda: self.select(provider)

    def mode_action(self, mode: str):
        return lambda: self.toggle_mode(mode)

    def sign_in_action(self, provider: str):
        return lambda: in_background(self.signin.ask, provider, True)

    def toggle_notifications(self) -> None:
        self.notifications.enabled = not self.notifications.enabled
        log.info("notifications %s", "on" if self.notifications.enabled else "off")
        self.persistence.save()
        self.ui.refresh_menu()

    def refresh_now(self) -> None:
        self.usage.reset_backoff()
        self.wake.set()

    def toggle_agent_events(self) -> None:
        self.notifications.agent_events = not self.notifications.agent_events
        log.info("agent-finished notifications %s", "on" if self.notifications.agent_events else "off")
        self.persistence.save()
        self.ui.refresh_menu()

    def quit(self) -> None:
        self.stop.set()
        self.wake.set()
        self.ui.stop()

    def run(self) -> None:
        threading.Thread(target=self.scheduler.run, daemon=True).start()
        threading.Thread(target=self.power.watch_lock, daemon=True).start()
        threading.Thread(target=self.agents.watch, daemon=True).start()
        self.ui.run()
