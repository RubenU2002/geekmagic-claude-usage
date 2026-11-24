"""Pausing by hand, and the computer being locked."""
import threading
import time
import unittest
from unittest.mock import patch

from geekmagic.application import config, tray_controller
from tests.support import AppTestCase, capture_screen, fake_fetchers, usage


class PowerTests(AppTestCase):
    def run_worker_briefly(self, app, seconds=0.4):
        """Let the app's loop run for a moment with the device and the screens faked out (the device answers)."""
        self.locator.reachable = True
        thread = threading.Thread(target=app.scheduler.run, daemon=True)
        with patch.object(config, "PAUSE_POLL", 0.05), capture_screen():
            thread.start()
            time.sleep(seconds)
            app.stop.set()
            app.wake.set()
            thread.join(2)

    def test_nothing_is_read_or_uploaded_while_paused(self):
        app = self.make()
        calls = []
        with fake_fetchers(claude=lambda: calls.append("claude") or usage(),
                           codex=lambda: calls.append("codex") or usage(title="Codex")):
            app.power.toggle_pause()
            self.assertTrue(app.power.paused)
            self.assertIn("en pausa", app.tooltip())
            self.run_worker_briefly(app)
        self.assertEqual(calls, [])

    def test_resuming_catches_up_at_once(self):
        app = self.make()
        app.power.paused = True
        app.power.toggle_pause()
        self.assertFalse(app.power.paused)
        self.assertTrue(app.wake.is_set())
        self.assertNotIn("pausa", app.tooltip())

    def test_the_updates_run_normally_when_not_paused(self):
        app = self.make()
        calls = []
        with fake_fetchers(claude=lambda: calls.append("claude") or usage(),
                           codex=lambda: calls.append("codex") or usage(title="Codex")):
            self.run_worker_briefly(app)
        self.assertIn("claude", calls)

    def test_locking_the_computer_pauses_unless_that_is_turned_off(self):
        app = self.make()
        self.assertTrue(app.power.pause_on_lock)
        app.power.set_locked(True)
        self.assertTrue(app.power.is_paused())
        self.assertIn("PC bloqueado", app.tooltip())
        calls = []
        with fake_fetchers(claude=lambda: calls.append("claude") or usage()):
            self.run_worker_briefly(app)
        self.assertEqual(calls, [], "locked: no queries")
        app.power.toggle_pause_on_lock()
        self.assertFalse(app.power.is_paused(), "locked but 'pause when locked' is off: keeps running")
        self.assertFalse(self.make().power.pause_on_lock, "remembered")

    def test_unlocking_wakes_the_loop_and_puts_the_view_back(self):
        app = self.make()
        app.screen.slots["claude"] = "a"
        app.power.set_locked(True)
        app.wake.clear()
        with capture_screen() as cap, patch.object(tray_controller, "in_background", lambda f, *a: f(*a)):
            app.power.set_locked(False)
        self.assertTrue(app.wake.is_set())
        self.assertEqual(cap.shown, ["claude-usage-a.gif"])

    def test_the_lock_watcher_notices_changes(self):
        app = self.make()
        states = iter([False, True, True, False])
        seen = []
        real_set = app.power.set_locked

        def is_locked():
            try:
                return next(states)
            except StopIteration:
                app.stop.set()
                return False

        self.lock_sensor.is_locked = is_locked
        with patch.object(config, "LOCK_POLL", 0.01), \
                patch.object(app.power, "set_locked", side_effect=lambda v: (seen.append(v), real_set(v))):
            app.power.watch_lock()
        self.assertEqual(seen, [True, False])


if __name__ == "__main__":
    unittest.main()
