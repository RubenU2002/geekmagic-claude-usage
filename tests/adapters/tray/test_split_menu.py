"""Where the split view's item sits in the tray menu (needs pystray; skipped without it)."""
import unittest

try:
    from geekmagic.adapters.tray.menu import build_menu
except ImportError:  # no pystray here
    build_menu = None

from tests.support import AppTestCase


@unittest.skipIf(build_menu is None, "pystray is not installed")
class SplitMenuTests(AppTestCase):
    def test_menu_has_the_split_item_next_to_the_other_buttons(self):
        app = self.make()
        names = [str(item) for item in build_menu(app, open_logs=lambda: None).items]
        self.assertIn("Vista dividida", names)
        self.assertLess(names.index("Codex"), names.index("Vista dividida"))
        self.assertLess(names.index("Vista dividida"), names.index("Ver logs"))


if __name__ == "__main__":
    unittest.main()
