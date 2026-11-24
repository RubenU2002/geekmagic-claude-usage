"""Telling "signed out" apart from other read failures, and the screen that asks for the sign-in."""
import json
import subprocess
import unittest
from dataclasses import replace
from unittest.mock import patch

from geekmagic.adapters.providers import claude
from geekmagic.adapters.render.views import error as error_view
from geekmagic.core.errors import SignInNeeded, UsageError
from geekmagic.core.model import ErrorScreen


class SignedOutDetectionTests(unittest.TestCase):
    def run_claude(self, stdout="", stderr="", returncode=0):
        done = subprocess.CompletedProcess([], returncode, stdout=stdout, stderr=stderr)
        with patch.object(claude.subprocess, "run", return_value=done):
            return claude.fetch_usage()

    def test_claude_signed_out_is_told_apart_from_other_failures(self):
        for text in ("Not logged in · Please run /login", "Invalid API key · Please run /login", "authentication_error"):
            with self.assertRaises(SignInNeeded, msg=text):
                self.run_claude(stderr=text, returncode=1)
            with self.assertRaises(SignInNeeded, msg=text):
                self.run_claude(stdout=json.dumps({"is_error": True, "result": text, "total_cost_usd": 0}))
        with self.assertRaises(UsageError) as caught:
            self.run_claude(stderr="segmentation fault", returncode=139)
        self.assertNotIsInstance(caught.exception, SignInNeeded)
        with self.assertRaises(UsageError) as caught:
            self.run_claude(stdout=json.dumps({"is_error": True, "result": "something odd", "total_cost_usd": 0}))
        self.assertNotIsInstance(caught.exception, SignInNeeded)

    def test_the_error_screen_asks_for_the_sign_in(self):
        from datetime import datetime
        screen = ErrorScreen("Codex", "Codex could not read account limits. Not signed in?", datetime.now().astimezone())
        plain = error_view.render_error_frame(screen)
        asking = error_view.render_error_frame(replace(screen, signin=True))
        self.assertEqual(plain.size, (240, 240))
        self.assertNotEqual(plain.tobytes(), asking.tobytes(), "the heading says what to do")


if __name__ == "__main__":
    unittest.main()
