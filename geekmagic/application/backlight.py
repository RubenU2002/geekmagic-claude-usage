"""The device's backlight: brightness, its own night schedule, and dimming while the computer is locked."""

from __future__ import annotations

import logging
import time
from collections.abc import Callable

from geekmagic.application import config
from geekmagic.application.screen import Screen
from geekmagic.application.threads import in_background

log = logging.getLogger("tray")


def hour12(hour: int) -> str:
    """22 -> "10 PM"."""
    return f"{hour % 12 or 12} {'AM' if hour % 24 < 12 else 'PM'}"


class Backlight:
    def __init__(self, screen: Screen, notify: Callable[[str, str], None], save: Callable[[], None],
                 refresh_menu: Callable[[], None], night_hours: tuple[int, int] | None = None,
                 night_level: int | None = None) -> None:
        self.screen = screen
        self.notify = notify
        self.save = save
        self.refresh_menu = refresh_menu
        self.brightness: int | None = None  # the day level we last set; the device can't be asked
        self.night: dict = dict(config.NIGHT_DEFAULT)  # the device's own night schedule, see Screen.device.set_night_mode
        self.dim_on_lock = False  # turn the screen's backlight down while the computer is locked
        self._command_line = (night_hours, night_level)

    def restore(self, saved: dict) -> None:
        self.dim_on_lock = bool(saved.get("dim_on_lock", False))
        level = saved.get("brightness")
        self.brightness = level if isinstance(level, int) and not isinstance(level, bool) else None
        night = saved.get("night")
        if isinstance(night, dict):
            self.night = {**config.NIGHT_DEFAULT, **{k: v for k, v in night.items() if k in config.NIGHT_DEFAULT}}
        night_hours, night_level = self._command_line  # --night START-END / --night-brightness: what the menu's night mode uses
        if night_hours:
            self.night["start"], self.night["end"] = night_hours
        if night_level is not None:
            self.night["level"] = night_level

    def snapshot(self) -> dict:
        return {"brightness": self.brightness, "night": self.night, "dim_on_lock": self.dim_on_lock}

    def apply_brightness(self, level: int, quiet: bool = False) -> bool:
        """Send a backlight level to the device. The device serves one request at a time, so an upload in flight is
        cancelled first (the worker redoes it). `quiet`: it isn't something you asked for (the dimming when the computer
        locks), so a screen that doesn't answer is logged but not announced."""
        if not self.screen.ip:
            return False
        if quiet and self.screen.offline:
            log.info("not setting the brightness: the screen isn't answering")
            return False
        self.screen.cancel.set()
        try:
            with self.screen.push_lock:
                self.screen.device.set_brightness(level)
            return True
        except OSError as e:
            log.warning("could not set the brightness: %s", e)
            if not quiet:
                self.notify("Brillo", f"No se pudo cambiar el brillo: {e}")
            return False

    def set_brightness(self, level: int) -> None:
        """Menu: the normal (day) brightness. Remembered, because the device can't tell us what it's set to."""
        log.info("brightness -> %s", level)
        self.screen.last_click = time.monotonic()  # leave the device free for this request
        if self.apply_brightness(level):
            self.brightness = level
            self.save()
            self.refresh_menu()
            if self.night["enabled"]:  # the schedule carries the day level too, keep it in step
                in_background(self.apply_night)

    def night_label(self) -> str:
        n = self.night
        return f"Modo nocturno ({hour12(n['start'])} - {hour12(n['end'])}, brillo {n['level']} %)"

    def apply_night(self) -> bool:
        if not self.screen.ip or self.brightness is None:
            return False
        self.screen.cancel.set()
        try:
            with self.screen.push_lock:
                self.screen.device.set_night_mode(
                    start_hour=self.night["start"], end_hour=self.night["end"], night_level=self.night["level"],
                    day_level=self.brightness, enabled=self.night["enabled"])
            return True
        except OSError as e:
            log.warning("could not set the night mode: %s", e)
            self.notify("Modo nocturno", f"No se pudo cambiar el modo nocturno: {e}")
            return False

    def need_day_brightness(self, what: str) -> bool:
        """Night mode and dimming restore the normal brightness afterwards, and the device can't report it: it has
        to be one we set. Ask for that first instead of guessing and changing it behind the user's back."""
        if self.brightness is not None:
            return True
        self.notify(what, "Elige primero tu brillo normal en Pantalla > Brillo, para que pueda restaurarlo.")
        return False

    def toggle_night(self) -> None:
        """Menu: the device's own night schedule (works even with the computer off)."""
        enable = not self.night["enabled"]
        if enable and not self.need_day_brightness("Modo nocturno"):
            return
        self.screen.last_click = time.monotonic()
        self.night["enabled"] = enable
        if self.apply_night():
            log.info("night mode %s", "on" if enable else "off")
        else:
            self.night["enabled"] = not enable  # the device didn't take it
        self.save()
        self.refresh_menu()

    def toggle_dim_on_lock(self) -> None:
        if not self.dim_on_lock and not self.need_day_brightness("Atenuar al bloquear"):
            return
        self.dim_on_lock = not self.dim_on_lock
        log.info("dim when locked %s", "on" if self.dim_on_lock else "off")
        self.save()
        self.refresh_menu()

    def on_lock_change(self, locked: bool) -> None:
        """The computer got locked or unlocked: turn the backlight down, or back to the day level."""
        if self.dim_on_lock and self.brightness is not None:
            in_background(self.apply_brightness, config.LOCK_DIM_LEVEL if locked else self.brightness, True)
