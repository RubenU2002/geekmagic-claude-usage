"""The tray menu: its items and their order, and the night-mode settings that come from the command line."""
import argparse
import unittest
from unittest.mock import MagicMock, patch

from geekmagic.application import backlight, config
from geekmagic.tray_main import night_hours
from tests.support import AppTestCase

try:
    from geekmagic.adapters.tray.menu import build_menu
except ImportError as error:  # pystray (or what it needs from the desktop) isn't available here
    build_menu = None
    MENU_UNAVAILABLE = f"the tray menu needs pystray: {error}"
else:
    MENU_UNAVAILABLE = ""


@unittest.skipIf(build_menu is None, MENU_UNAVAILABLE)
class MenuTests(AppTestCase):
    def build(self, app):
        return build_menu(app, lambda: None)

    def test_the_menu_has_every_new_button_in_a_sensible_order(self):
        app = self.make()
        menu = self.build(app)
        top = [item.text for item in menu.items if not item.text.startswith("-")]  # (no separators)
        for wanted in ("Vista dividida", "Estadísticas", "Más vistas", "Actualizar ahora", "Pausar", "Pantalla", "Opciones", "Ver logs", "Salir"):
            self.assertIn(wanted, top)
        self.assertLess(top.index("Vista dividida"), top.index("Estadísticas"))
        self.assertLess(top.index("Estadísticas"), top.index("Actualizar ahora"))
        self.assertLess(top.index("Opciones"), top.index("Ver logs"))
        sub = {i.text: [s.text for s in i.submenu.items if not s.text.startswith("-")]
               for i in menu.items if i.submenu}
        self.assertEqual([x for x in sub["Pantalla"] if x.startswith("Brillo")],
                         [f"Brillo {level} %" for level in config.DEFAULT_BRIGHTNESS_CHOICES])
        self.assertIn("Atenuar al bloquear el PC", sub["Pantalla"])
        self.assertTrue(any(x.startswith("Modo nocturno") for x in sub["Pantalla"]))
        self.assertEqual(sub["Opciones"], ["Iniciar sesión", "Notificaciones", "Avisar cuando un agente termine o te espere", "Pausar al bloquear el PC"])
        self.assertEqual(sub["Más vistas"], ["Proyectos y modelos", "Horas pico"])

    def test_there_is_a_provider_item_for_each_provider_the_app_knows(self):
        app = self.make()
        top = [item.text for item in self.build(app).items]
        for title in app.providers.titles.values():
            self.assertIn(title, top)

    def test_the_sign_in_submenu_has_every_provider(self):
        app = self.make()
        options = next(i for i in self.build(app).items if i.text == "Opciones")
        sign_in = next(i for i in options.submenu.items if i.text == "Iniciar sesión")
        self.assertEqual([i.text for i in sign_in.submenu.items], list(app.providers.titles.values()))

    def test_ver_logs_runs_the_command_the_ui_gave(self):
        opened = []
        menu = build_menu(self.make(), lambda: opened.append(True))
        # (the item's action is the command itself: pystray calls it with the icon and the item as needed)
        item = next(i for i in menu.items if i.text == "Ver logs")
        item(MagicMock())
        self.assertEqual(opened, [True])

    def test_brightness_menu_items_act_on_their_own_level(self):
        app = self.make()
        patcher = patch.object(backlight, "in_background", lambda f, *a: f(*a))  # run the work handed to a thread right away
        patcher.start()
        self.addCleanup(patcher.stop)
        screen = next(i for i in self.build(app).items if i.text == "Pantalla")
        item = next(i for i in screen.submenu.items if i.text == "Brillo 25 %")
        item(MagicMock())
        self.assertIn(("brightness", 25), app.screen.device.calls)
        self.assertTrue(item.checked)


class CommandLineNightTests(AppTestCase):
    def test_night_hours_argument(self):
        self.assertEqual(night_hours("22-7"), (22, 7))
        self.assertEqual(night_hours("0-23"), (0, 23))
        for bad in ("22", "24-7", "a-b", "22-7-9", "-3-5", ""):
            with self.assertRaises(argparse.ArgumentTypeError, msg=bad):
                night_hours(bad)

    def test_night_settings_from_the_command_line_feed_the_menu_item(self):
        app = self.make(night_hours=(23, 6), night_level=5)
        night = app.backlight.night
        self.assertEqual((night["start"], night["end"], night["level"]), (23, 6, 5))
        self.assertFalse(night["enabled"], "defining the schedule doesn't switch it on")
        self.assertEqual(app.backlight.night_label(), "Modo nocturno (11 PM - 6 AM, brillo 5 %)")


if __name__ == "__main__":
    unittest.main()
