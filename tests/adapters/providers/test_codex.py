"""Exercise the real stdio client against a controlled server, without credentials."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from geekmagic.adapters.providers import codex
from geekmagic.adapters.providers import executables
from geekmagic.adapters.render.views import single
from geekmagic.core.errors import SignInNeeded, UsageError
from geekmagic.core.model import CodexExtra


class CodexUsageTests(unittest.TestCase):
    def query(self, result=None, mode="success", timeout=5):
        server = '''
import json, sys, time
result = json.loads(sys.argv[1])
mode = sys.argv[2]
hello = json.loads(sys.stdin.readline())
assert hello["method"] == "initialize"
if mode == "exit":
    sys.exit(0)
print(json.dumps({"id": hello["id"], "result": {}}), flush=True)
assert json.loads(sys.stdin.readline())["method"] == "initialized"
request = json.loads(sys.stdin.readline())
assert request["method"] == "account/rateLimits/read"
if mode == "timeout":
    time.sleep(60)
elif mode == "error":
    print(json.dumps({"id": request["id"], "error": {"message": "not authenticated"}}), flush=True)
else:
    print(json.dumps({"method": "account/rateLimits/updated", "params": {}}), flush=True)
    print(json.dumps({"id": request["id"], "result": result}), flush=True)
assert sys.stdin.read() == ""  # No threads, prompts, or model turns were sent.
'''
        processes = []
        real_popen = subprocess.Popen
        with tempfile.TemporaryDirectory() as temp:
            script = Path(temp) / "server.py"
            script.write_text(server, encoding="utf-8")

            def launch(command, **kwargs):
                self.assertEqual(command, ["fake-codex", "app-server"])
                proc = real_popen([sys.executable, str(script), json.dumps(result), mode], **kwargs)
                processes.append(proc)
                return proc

            try:
                with patch.object(executables.shutil, "which", return_value="fake-codex"), \
                        patch.object(codex.subprocess, "Popen", side_effect=launch), \
                        patch.object(codex, "CODEX_TIMEOUT", timeout):
                    return codex.fetch_codex_usage()
            finally:
                for proc in processes:
                    self.assertIsNotNone(proc.poll(), "App-server child must be reaped")
                    self.assertTrue(proc.stdin.closed)
                    self.assertTrue(proc.stdout.closed)

    def test_fresh_queries_and_codex_bucket(self):
        for percent in (25, 31):
            result = self.query({
                "rateLimits": {"primary": {"usedPercent": 99}},
                "rateLimitsByLimitId": {"codex": {
                    "primary": {"usedPercent": percent, "resetsAt": 1900000000},
                    "secondary": {"usedPercent": 42, "resetsAt": 1900600000},
                }},
            })
            self.assertEqual(result.current.pct, percent)
            self.assertEqual(result.weekly.pct, 42)
            self.assertEqual(result.current.reset.timestamp(), 1900000000)
            self.assertTrue(single.render_animation(result).startswith(b"GIF"))

    def test_secondary_only_and_server_percent_at_past_reset(self):
        result = self.query({"rateLimits": {"secondary": {"usedPercent": 17, "resetsAt": 1700000000}}})
        self.assertIsNone(result.current.pct)
        self.assertEqual(result.weekly.pct, 17)  # Do not invent a 0% reading.
        self.assertTrue(single.render_animation(result).startswith(b"GIF"))

    def test_missing_cli(self):
        with patch.object(codex, "find_codex", return_value=None):
            with self.assertRaisesRegex(UsageError, "not found"):
                codex.fetch_codex_usage()

    def test_auth_failure_and_no_limits(self):
        with self.assertRaisesRegex(SignInNeeded, "codex login"):  # being signed out is what signing in again fixes
            self.query(mode="error")
        with self.assertRaisesRegex(SignInNeeded, "no account limits"):
            self.query({"rateLimits": None})

    def test_child_exit_and_timeout(self):
        with self.assertRaisesRegex(UsageError, "exited"):
            self.query(mode="exit")
        start = time.monotonic()
        with self.assertRaisesRegex(UsageError, "Timed out"):
            self.query(mode="timeout", timeout=0.25)
        self.assertLess(time.monotonic() - start, 5)

    def test_invalid_window(self):
        with self.assertRaisesRegex(UsageError, "invalid quota window"):
            self.query({"rateLimits": {"primary": {"resetsAt": 1900000000}}})


class CodexExtraTests(unittest.TestCase):
    """The free rate-limit resets Codex has granted: how many are left and when the next one expires."""

    def extra(self, count=3, days=12, status="available"):
        return codex._codex_extra({"_reset_credits": {"availableCount": count, "credits": [
            {"status": status, "expiresAt": time.time() + days * 86400},
            {"status": "available", "expiresAt": time.time() + (days + 30) * 86400},
        ]}})

    def test_a_null_or_odd_credits_list_is_not_a_crash(self):
        """Codex once sent `credits: null` and the whole read failed with a TypeError."""
        for credits in (None, "none", 5, {}):
            extra = codex._codex_extra({"_reset_credits": {"availableCount": 2, "credits": credits}})
            self.assertEqual((extra.free_resets, extra.next_reset_expires_days), (2, None), repr(credits))

    def test_free_resets_and_when_the_next_one_expires(self):
        extra = self.extra()
        self.assertEqual((extra.free_resets, extra.next_reset_expires_days), (3, 12))

    def test_used_ones_do_not_count_towards_the_expiry(self):
        self.assertEqual(self.extra(days=1, status="used").next_reset_expires_days, 31)

    def test_missing_fields_are_fine(self):
        self.assertEqual(codex._codex_extra({}), CodexExtra(free_resets=0, next_reset_expires_days=None))
        self.assertEqual(codex._codex_extra({"_reset_credits": "garbage"}), CodexExtra(free_resets=0, next_reset_expires_days=None))


if __name__ == "__main__":
    unittest.main()
