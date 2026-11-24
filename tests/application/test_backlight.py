"""The device's backlight: brightness, its night schedule, and dimming while the computer is locked."""
import unittest
from unittest.mock import Mock, patch

from geekmagic.application import backlight, config, tray_controller
from tests.support import AppTestCase, capture_screen


class BacklightTests(AppTestCase):
    def sync_background(self):
        """Run the work the app hands to a thread right away, so the test can look at the outcome."""
        for module in (backlight, tray_controller):
            patcher = patch.object(module, "in_background", lambda f, *a: f(*a))
            patcher.start()
            self.addCleanup(patcher.stop)

    @staticmethod
    def on_device(app, method, **behaviour):
        """Replace one of the screen's methods with a recorder (`side_effect=OSError(..)` makes it refuse)."""
        recorder = Mock(**behaviour)
        setattr(app.screen.device, method, recorder)
        return recorder

    def test_brightness_is_sent_remembered_and_marked_in_the_menu(self):
        app = self.make()
        self.sync_background()
        send = self.on_device(app, "set_brightness")
        app.backlight.set_brightness(75)
        send.assert_called_once_with(75)
        self.assertEqual(app.backlight.brightness, 75)
        self.assertEqual(self.make().backlight.brightness, 75, "remembered across restarts")

    def test_a_refused_brightness_changes_nothing(self):
        app = self.make()
        self.on_device(app, "set_brightness", side_effect=OSError("device refused"))
        app.backlight.set_brightness(75)
        self.assertIsNone(app.backlight.brightness)
        self.assertIn("No se pudo cambiar el brillo", self.notes[-1][1])

    def test_night_mode_needs_a_known_day_brightness_first(self):
        app = self.make()
        send = self.on_device(app, "set_night_mode")
        app.backlight.toggle_night()
        send.assert_not_called()
        self.assertFalse(app.backlight.night["enabled"])
        self.assertIn("Elige primero tu brillo normal", self.notes[-1][1])

    def test_night_mode_sends_the_schedule_with_the_day_level(self):
        app = self.make()
        app.backlight.brightness = 60
        send = self.on_device(app, "set_night_mode")
        app.backlight.toggle_night()
        send.assert_called_once_with(start_hour=22, end_hour=7, night_level=10, day_level=60, enabled=True)
        self.assertTrue(app.backlight.night["enabled"])
        app.backlight.toggle_night()
        self.assertFalse(send.call_args.kwargs["enabled"])
        self.assertFalse(app.backlight.night["enabled"])

    def test_night_mode_stays_off_if_the_device_refuses(self):
        app = self.make()
        app.backlight.brightness = 60
        self.on_device(app, "set_night_mode", side_effect=OSError("FAIL"))
        app.backlight.toggle_night()
        self.assertFalse(app.backlight.night["enabled"])

    def test_changing_the_day_brightness_keeps_an_enabled_night_schedule_in_step(self):
        app = self.make()
        self.sync_background()
        app.backlight.brightness, app.backlight.night["enabled"] = 60, True
        self.on_device(app, "set_brightness")
        night = self.on_device(app, "set_night_mode")
        app.backlight.set_brightness(30)
        self.assertEqual(night.call_args.kwargs["day_level"], 30)

    def test_night_label_and_settings_round_trip(self):
        app = self.make()
        self.assertEqual(app.backlight.night_label(), "Modo nocturno (10 PM - 7 AM, brillo 10 %)")
        self.assertEqual([backlight.hour12(h) for h in (0, 7, 12, 13, 22, 23)], ["12 AM", "7 AM", "12 PM", "1 PM", "10 PM", "11 PM"])
        app.backlight.night.update(enabled=True, start=23, end=6, level=5)
        app.backlight.brightness = 80
        app.persistence.save()
        again = self.make()
        self.assertEqual((again.backlight.night, again.backlight.brightness), ({"enabled": True, "start": 23, "end": 6, "level": 5}, 80))

    def test_dim_when_locked_dims_then_restores_the_known_brightness(self):
        app = self.make()
        self.sync_background()
        app.backlight.brightness = 70
        send = self.on_device(app, "set_brightness")
        with capture_screen():
            app.backlight.toggle_dim_on_lock()
            self.assertTrue(app.backlight.dim_on_lock)
            app.power.set_locked(True)
            app.power.set_locked(False)
        self.assertEqual([c.args[0] for c in send.call_args_list], [config.LOCK_DIM_LEVEL, 70])

    def test_the_automatic_dimming_stays_quiet_when_the_screen_does_not_answer(self):
        app = self.make()
        app.backlight.brightness = 70
        send = self.on_device(app, "set_brightness", side_effect=OSError("timed out"))
        app.screen.offline = True
        self.assertFalse(app.backlight.apply_brightness(config.LOCK_DIM_LEVEL, quiet=True))
        self.assertEqual((send.call_count, self.notes), (0, []))
        app.screen.offline = False
        self.assertFalse(app.backlight.apply_brightness(config.LOCK_DIM_LEVEL, quiet=True))
        self.assertEqual(self.notes, [], "failed, but nobody asked for it")
        self.assertFalse(app.backlight.apply_brightness(40))
        self.assertIn("No se pudo cambiar el brillo", self.notes[-1][1])  # a menu click does say so

    def test_dim_when_locked_refuses_to_guess_the_brightness_to_restore(self):
        app = self.make()
        app.backlight.toggle_dim_on_lock()
        self.assertFalse(app.backlight.dim_on_lock)
        self.assertIn("Elige primero tu brillo normal", self.notes[-1][1])

    def test_no_dimming_when_it_is_off_or_there_is_no_device_address(self):
        app = self.make()
        self.sync_background()
        app.backlight.brightness = 70
        send = self.on_device(app, "set_brightness")
        with capture_screen():
            app.power.set_locked(True)  # dim_on_lock is off
            self.assertEqual(send.call_count, 0)
            app.backlight.dim_on_lock = True
            app.screen.set_ip_quietly("")  # no device address (the old test blanked `app.ip`)
            app.power.set_locked(False)
            self.assertEqual(send.call_count, 0)


if __name__ == "__main__":
    unittest.main()
