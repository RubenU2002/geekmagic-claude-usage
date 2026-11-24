"""Opening a terminal window that runs a provider's sign-in."""
import os
import unittest
from unittest.mock import patch

from geekmagic.adapters.system import terminal


class TerminalCommandTests(unittest.TestCase):
    def test_windows_gets_one_command_line_that_survives_spaces_in_the_path(self):
        command = terminal.terminal_command([r"C:\Program Files\Claude Code\claude.exe", "auth", "login"], "Claude sign-in", "win32")
        self.assertEqual(command, r'cmd.exe /c start "Claude sign-in" cmd.exe /k ""C:\Program Files\Claude Code\claude.exe" auth login"')

    def test_windows_command_without_spaces(self):
        command = terminal.terminal_command(["codex.cmd"], "Codex sign-in", "win32")
        self.assertEqual(command, 'cmd.exe /c start "Codex sign-in" cmd.exe /k "codex.cmd"')

    def test_macos_opens_terminal_and_quotes_what_it_runs(self):
        command = terminal.terminal_command(["/Users/me/my tools/claude", "auth", "login"], "t", "darwin")
        self.assertEqual(command[:2], ["osascript", "-e"])
        script = command[-1]
        self.assertIn('do script "', script)
        self.assertIn("'/Users/me/my tools/claude' auth login", script)

    def test_macos_script_cannot_be_broken_out_of(self):
        command = terminal.terminal_command(['/tmp/x"; do shell script "rm -rf ~"; "'], "t", "darwin")
        body = command[-1].split('do script "', 1)[1]
        self.assertNotIn('rm -rf ~"; "', body.replace('\\"', ""), "every quote inside the command is escaped")

    def test_linux_uses_the_first_terminal_it_finds(self):
        found = {"gnome-terminal", "xterm"}
        with patch.object(terminal.shutil, "which", side_effect=lambda name: f"/usr/bin/{name}" if name in found else None):
            self.assertEqual(terminal.terminal_command(["codex"], "t", "linux"), ["gnome-terminal", "--", "codex"])
        with patch.object(terminal.shutil, "which", return_value=None):
            self.assertIsNone(terminal.terminal_command(["codex"], "t", "linux"))


class LaunchTests(unittest.TestCase):
    def setUp(self):
        patcher = patch.dict(os.environ, {"GEEKMAGIC_NO_LOGIN": ""})  # (the suite turns it on; these tests are about it)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_the_kill_switch_opens_nothing(self):
        with patch.dict(os.environ, {"GEEKMAGIC_NO_LOGIN": "1"}), patch.object(terminal.subprocess, "Popen") as popen:
            self.assertEqual(terminal.launch(["claude", "auth", "login"], "Claude sign-in"), "failed")
        popen.assert_not_called()

    def test_it_opens_the_terminal_with_the_command_it_is_given(self):
        with patch.object(terminal, "terminal_command", return_value=["term", "codex", "login"]) as build, \
                patch.object(terminal.subprocess, "Popen") as popen:
            self.assertEqual(terminal.launch(["/bin/codex", "login"], "Codex sign-in"), "started")
        build.assert_called_once_with(["/bin/codex", "login"], "Codex sign-in")
        self.assertEqual(popen.call_args.args[0], ["term", "codex", "login"])

    def test_no_terminal_or_a_failing_start_is_reported(self):
        with patch.object(terminal, "terminal_command", return_value=None):
            self.assertEqual(terminal.launch(["claude"], "t"), "failed")
        with patch.object(terminal, "terminal_command", return_value=["term"]), \
                patch.object(terminal.subprocess, "Popen", side_effect=OSError("no")):
            self.assertEqual(terminal.launch(["claude"], "t"), "failed")


if __name__ == "__main__":
    unittest.main()
