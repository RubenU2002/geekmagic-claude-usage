"""Starting at login: the entries are named after the app (not after one provider), and an old install is replaced."""
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from geekmagic.adapters.system import autostart


class NamingTests(unittest.TestCase):
    def test_nothing_is_named_after_a_single_provider_any_more(self):
        for name in (autostart.TASK_NAME, autostart.LABEL, autostart.DESKTOP_FILE):
            self.assertNotIn("claude", name.lower())
        self.assertEqual(autostart.LEGACY_TASK_NAME, "GeekMagicClaude", "what the old installs are called")

    def test_the_mac_plist_and_the_desktop_entry_use_the_new_names(self):
        plist = autostart.build_mac_plist(["python", "tray.py"], Path("/app"))
        self.assertIn(b"com.geekmagic.usage", plist)
        self.assertIn("GeekMagic usage", autostart.build_desktop_entry(["python", "tray.py"], Path("/app")))
        self.assertEqual(autostart.mac_plist_path().name, "com.geekmagic.usage.plist")
        self.assertEqual(autostart.linux_desktop_path().name, "geekmagic-usage.desktop")


class WindowsTaskTests(unittest.TestCase):
    def run_with_fake_powershell(self, action):
        scripts = []
        with patch.object(autostart, "_powershell", side_effect=scripts.append):
            action()
        return "\n".join(scripts)

    def test_installing_removes_the_old_task_before_creating_the_new_one(self):
        script = self.run_with_fake_powershell(lambda: autostart._install_windows(["pythonw", "tray.py"], Path("C:/app")))
        self.assertLess(script.index("Unregister-ScheduledTask -TaskName GeekMagicClaude"),
                        script.index("Register-ScheduledTask -TaskName GeekMagicUsage"))

    def test_uninstalling_removes_both(self):
        script = self.run_with_fake_powershell(autostart._uninstall_windows)
        self.assertIn("Unregister-ScheduledTask -TaskName GeekMagicUsage", script)
        self.assertIn("Unregister-ScheduledTask -TaskName GeekMagicClaude", script)


class LinuxEntryTests(unittest.TestCase):
    def test_installing_replaces_an_old_entry_and_uninstalling_removes_both(self):
        with tempfile.TemporaryDirectory() as home, patch.dict("os.environ", {"XDG_CONFIG_HOME": home}):
            folder = Path(home) / "autostart"
            folder.mkdir()
            old = folder / autostart.LEGACY_DESKTOP_FILE
            old.write_text("old", encoding="utf-8")
            autostart._install_linux(["python", "tray.py"], Path("/app"))
            self.assertFalse(old.exists())
            self.assertTrue((folder / autostart.DESKTOP_FILE).exists())
            old.write_text("old", encoding="utf-8")
            autostart._uninstall_linux()
            self.assertEqual(list(folder.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
