"""Native desktop notifications: the right helper per OS, and nothing the user's text could break."""
import os
import subprocess
import unittest
from unittest.mock import MagicMock, patch

from geekmagic.adapters.system import notifier

NASTY = ['He said "hi"', "it's", "line one\nline two", "back`tick $env:PATH", "'; Remove-Item -Recurse C:\\ ; '", "<b>&amp;</b>",
         "a" * 2000, "emoji 🔔 and ñ ü"]


class CommandTests(unittest.TestCase):
    def test_windows_goes_through_a_powershell_toast_in_its_own_identity(self):
        argv, env = notifier.command("Claude · Acme App", "Te hizo una pregunta", "win32")
        self.assertEqual(argv[0], "powershell")
        self.assertIn("-NoProfile", argv)
        self.assertIn("-NonInteractive", argv)
        script = argv[-1]
        self.assertIn("ToastNotificationManager", script)
        self.assertIn("1AC14E77-02E7-4E5D-B744-2EB1AE5198B7", script, "PowerShell's own app identity, which Windows shows toasts for")
        self.assertEqual(env, {"GM_TITLE": "Claude · Acme App", "GM_MESSAGE": "Te hizo una pregunta"})

    def test_macos_uses_osascript(self):
        argv, env = notifier.command("Claude · Acme App", "Hola", "darwin")
        self.assertEqual(argv[:2], ["osascript", "-e"])
        self.assertIn("display notification", argv[2])
        self.assertEqual(env["GM_MESSAGE"], "Hola")

    def test_linux_uses_notify_send_with_the_text_as_arguments(self):
        argv, _ = notifier.command("Claude · Acme App", "Hola", "linux")
        self.assertEqual(argv[0], "notify-send")
        self.assertEqual(argv[-2:], ["Claude · Acme App", "Hola"])

    def test_other_systems_have_no_native_route(self):
        self.assertIsNone(notifier.command("t", "m", "plan9"))

    def test_the_text_never_ends_up_inside_a_command_string(self):
        for text in NASTY:
            for platform in ("win32", "darwin"):
                argv, env = notifier.command(text, text, platform)
                self.assertNotIn(text, " ".join(argv), f"{platform}: the text must travel in the environment")
                self.assertEqual((env["GM_TITLE"], env["GM_MESSAGE"]), (text, text))
            argv, _ = notifier.command(text, text, "linux")
            self.assertEqual(argv[-2:], [text, text], "as separate arguments (no shell involved)")

    def test_the_windows_script_is_fixed_whatever_the_text(self):
        scripts = {notifier.command(text, text, "win32")[0][-1] for text in NASTY}
        self.assertEqual(len(scripts), 1)


class SendTests(unittest.TestCase):
    def setUp(self):
        patcher = patch.dict(os.environ)
        patcher.start()
        self.addCleanup(patcher.stop)
        os.environ.pop("GEEKMAGIC_NO_NOTIFY", None)  # (the test package sets it; here the real code path is the point)

    def test_it_starts_the_helper_with_the_text_in_the_environment_and_no_window(self):
        proc = MagicMock()
        with patch.object(notifier.subprocess, "Popen", return_value=proc) as popen, patch.object(notifier.threading, "Thread"):
            self.assertTrue(notifier.send("Claude · Acme App", 'say "hi"\nplease'))
        args, kwargs = popen.call_args
        self.assertIsInstance(args[0], list, "an argument list: no shell")
        self.assertNotIn("shell", kwargs)
        self.assertEqual(kwargs["env"]["GM_TITLE"], "Claude · Acme App")
        self.assertEqual(kwargs["env"]["GM_MESSAGE"], 'say "hi"\nplease')
        self.assertIn("PATH", kwargs["env"], "the rest of the environment is kept")
        self.assertEqual(kwargs["creationflags"], getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.assertEqual((kwargs["stdin"], kwargs["stdout"], kwargs["stderr"]), (subprocess.DEVNULL,) * 3)

    def test_it_does_not_wait_for_the_helper(self):
        proc = MagicMock()
        with patch.object(notifier.subprocess, "Popen", return_value=proc), patch.object(notifier.threading, "Thread") as thread:
            notifier.send("t", "m")
        proc.wait.assert_not_called()
        proc.communicate.assert_not_called()
        thread.assert_called_once()  # a reaper thread does the waiting, so nothing is left a zombie
        self.assertTrue(thread.call_args.kwargs["daemon"])

    def test_a_missing_helper_returns_false_so_the_caller_can_fall_back(self):
        with patch.object(notifier.subprocess, "Popen", side_effect=FileNotFoundError("notify-send")):
            self.assertFalse(notifier.send("t", "m"))

    def test_a_platform_without_a_route_returns_false(self):
        with patch.object(notifier, "command", return_value=None):
            self.assertFalse(notifier.send("t", "m"))

    def test_the_kill_switch_sends_nothing_and_reports_success(self):
        os.environ["GEEKMAGIC_NO_NOTIFY"] = "1"
        with patch.object(notifier.subprocess, "Popen") as popen:
            self.assertTrue(notifier.send("t", "m"))
        popen.assert_not_called()


class ReaperTests(unittest.TestCase):
    def test_a_helper_that_hangs_is_killed(self):
        proc = MagicMock()
        proc.wait.side_effect = [subprocess.TimeoutExpired("x", 1), None]
        notifier._reap(proc)
        proc.kill.assert_called_once()

    def test_a_helper_that_finishes_is_just_collected(self):
        proc = MagicMock()
        notifier._reap(proc)
        proc.kill.assert_not_called()


@unittest.skipUnless(os.name == "nt", "the real toast is Windows-only")
class RealWindowsTests(unittest.TestCase):
    def test_the_powershell_script_runs_and_returns_success_for_awkward_text(self):
        """Not a mock: PowerShell really parses the script and builds the toast (the kill switch is only for send())."""
        for text in ('He said "hi"', "it's a ñ", "line one\nline two"):
            argv, extra = notifier.command(text, text, "win32")
            # build the toast XML and stop short of showing it, so the test is silent: swap the final Show() for a check
            script = argv[-1].rsplit(";", 1)[0] + "; if ($toast) { 'built' }"
            done = subprocess.run(argv[:-1] + [script], env={**os.environ, **extra}, capture_output=True, text=True, timeout=60,
                                  creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
            self.assertEqual(done.stdout.strip(), "built", done.stderr)


if __name__ == "__main__":
    unittest.main()
