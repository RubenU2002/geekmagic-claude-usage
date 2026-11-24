"""The tray icon's right-click menu (its labels are in Spanish)."""

from __future__ import annotations

import pystray

from geekmagic.application import config
from geekmagic.core.views import BREAKDOWN, HOURS, SPLIT, STATS


def build_menu(app, open_logs) -> pystray.Menu:
    """`app` is the TrayController whose state the checkmarks show and whose commands the items run; `open_logs` is the
    UI's own command for "Ver logs". The left-click and "Pantalla simulada" go through the app too."""
    view, power, backlight, notifications = app.view, app.power, app.backlight, app.notifications
    titles = app.providers.titles
    return pystray.Menu(
        pystray.MenuItem(lambda _: app.screen.status_line(), None, enabled=False),
        pystray.Menu.SEPARATOR,
        pystray.MenuItem("Alternar Claude / Codex", app.tray_click, default=True, visible=False),
        *(pystray.MenuItem(title, app.select_action(key), radio=True,
                           checked=lambda _, key=key: view.mode is None and view.provider == key)
          for key, title in titles.items()),
        pystray.MenuItem("Vista dividida", app.mode_action(SPLIT), checked=lambda _: view.mode == SPLIT),
        pystray.MenuItem("Estadísticas", app.mode_action(STATS), checked=lambda _: view.mode == STATS),
        pystray.MenuItem("Más vistas", pystray.Menu(
            pystray.MenuItem("Proyectos y modelos", app.mode_action(BREAKDOWN), checked=lambda _: view.mode == BREAKDOWN),
            pystray.MenuItem("Horas pico", app.mode_action(HOURS), checked=lambda _: view.mode == HOURS),
        )),
        pystray.Menu.SEPARATOR,
        pystray.MenuItem("Actualizar ahora", app.refresh_now),
        pystray.MenuItem("Pantalla simulada", app.open_simulator),
        pystray.MenuItem("Guardar pantallazo...", app.capture.take),
        pystray.MenuItem("Pausar", power.toggle_pause, checked=lambda _: power.paused),
        pystray.MenuItem("Pantalla", pystray.Menu(
            *(pystray.MenuItem(f"Brillo {level} %", _brightness_action(backlight, level),
                               checked=lambda _, level=level: backlight.brightness == level, radio=True)
              for level in config.DEFAULT_BRIGHTNESS_CHOICES),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem(lambda _: backlight.night_label(), backlight.toggle_night, checked=lambda _: backlight.night["enabled"]),
            pystray.MenuItem("Atenuar al bloquear el PC", backlight.toggle_dim_on_lock, checked=lambda _: backlight.dim_on_lock),
        )),
        pystray.MenuItem("Opciones", pystray.Menu(
            pystray.MenuItem("Iniciar sesión", pystray.Menu(
                *(pystray.MenuItem(title, app.sign_in_action(key)) for key, title in titles.items()))),
            pystray.MenuItem("Notificaciones", app.toggle_notifications, checked=lambda _: notifications.enabled),
            pystray.MenuItem("Avisar cuando un agente termine o te espere", app.toggle_agent_events,
                             checked=lambda _: notifications.agent_events),
            pystray.MenuItem("Pausar al bloquear el PC", power.toggle_pause_on_lock, checked=lambda _: power.pause_on_lock),
        )),
        pystray.MenuItem("Ver logs", open_logs),
        pystray.MenuItem("Salir", app.quit),
    )


def _brightness_action(backlight, level: int):
    return lambda: backlight.set_brightness(level)
