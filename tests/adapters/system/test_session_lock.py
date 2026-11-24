"""Screen-lock detection: real on this machine, mocked for the other platforms."""
import os
import subprocess
import sys
import unittest
from unittest.mock import MagicMock, patch

from geekmagic.adapters.system import session_lock


class IsLockedTests(unittest.TestCase):
    def test_an_unlocked_session_is_not_locked(self):
        # the tests run in a logged-in session; if this ever fails the machine really was locked
        self.assertIs(session_lock.is_locked(), False)

    def test_never_raises_whatever_the_platform_does(self):
        for platform in ("win32", "darwin", "linux"):
            with patch.object(sys, "platform", platform), \
                    patch.object(session_lock, "_locked_windows", side_effect=RuntimeError("boom")), \
                    patch.object(session_lock, "_locked_mac", side_effect=ImportError("no Quartz")), \
                    patch.object(session_lock, "_locked_linux", side_effect=OSError("no loginctl")):
                self.assertIs(session_lock.is_locked(), False, platform)

    def test_dispatches_per_platform(self):
        for platform, name in (("win32", "_locked_windows"), ("darwin", "_locked_mac"), ("linux", "_locked_linux")):
            with patch.object(sys, "platform", platform), patch.object(session_lock, name, return_value=True) as impl:
                self.assertIs(session_lock.is_locked(), True, platform)
                impl.assert_called_once()


class LinuxTests(unittest.TestCase):
    def run_loginctl(self, stdout, session="c1"):
        env = {"XDG_SESSION_ID": session} if session else {}
        with patch.dict(os.environ, env, clear=True), \
                patch.object(subprocess, "run", return_value=MagicMock(stdout=stdout)) as run:
            return session_lock._locked_linux(), run

    def test_locked_hint_yes_means_locked(self):
        locked, run = self.run_loginctl("yes\n")
        self.assertTrue(locked)
        self.assertEqual(run.call_args[0][0][:3], ["loginctl", "show-session", "c1"])

    def test_no_and_anything_else_means_not_locked(self):
        self.assertFalse(self.run_loginctl("no\n")[0])
        self.assertFalse(self.run_loginctl("")[0])

    def test_without_a_session_id_it_does_not_even_ask(self):
        locked, run = self.run_loginctl("yes\n", session=None)
        self.assertFalse(locked)
        run.assert_not_called()


class MacTests(unittest.TestCase):
    def test_reads_the_session_dictionary(self):
        quartz = MagicMock()
        with patch.dict(sys.modules, {"Quartz": quartz}):
            quartz.CGSessionCopyCurrentDictionary.return_value = {"CGSSessionScreenIsLocked": 1}
            self.assertTrue(session_lock._locked_mac())
            quartz.CGSessionCopyCurrentDictionary.return_value = {"kCGSSessionOnConsoleKey": 1}
            self.assertFalse(session_lock._locked_mac())
            quartz.CGSessionCopyCurrentDictionary.return_value = None
            self.assertFalse(session_lock._locked_mac())


if __name__ == "__main__":
    unittest.main()
