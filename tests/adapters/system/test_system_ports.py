"""The classes that put the system's functions behind the application's ports."""
import unittest
from unittest.mock import patch

from geekmagic.adapters.system import notifier, session_lock, terminal
from geekmagic.adapters.system.notifier import SystemNotifier
from geekmagic.adapters.system.session_lock import SessionLockSensor
from geekmagic.adapters.system.terminal import Terminal


class SystemNotifierTests(unittest.TestCase):
    def test_it_raises_when_the_system_has_no_native_route(self):
        # (the app then falls back to the tray icon's own notification, which is why this one must say it failed)
        with patch.object(notifier, "send", return_value=False) as send:
            with self.assertRaises(OSError):
                SystemNotifier().send("Claude", "80 % used")
        send.assert_called_once_with("Claude", "80 % used")

    def test_it_is_silent_when_the_notification_was_started(self):
        with patch.object(notifier, "send", return_value=True) as send:
            self.assertIsNone(SystemNotifier().send("Claude", "80 % used"))
        send.assert_called_once_with("Claude", "80 % used")


class SessionLockSensorTests(unittest.TestCase):
    def test_it_reports_what_the_module_finds(self):
        for locked in (True, False):
            with patch.object(session_lock, "is_locked", return_value=locked):
                self.assertIs(SessionLockSensor().is_locked(), locked)


class TerminalTests(unittest.TestCase):
    def test_it_launches_the_command_it_is_given(self):
        with patch.object(terminal, "launch", return_value="started") as launch:
            self.assertEqual(Terminal().launch(["/bin/codex", "login"], "Codex sign-in"), "started")
        launch.assert_called_once_with(["/bin/codex", "login"], "Codex sign-in")

    def test_it_passes_a_failure_on(self):
        with patch.object(terminal, "launch", return_value="failed"):
            self.assertEqual(Terminal().launch(["codex"], "t"), "failed")


if __name__ == "__main__":
    unittest.main()
