"""Clicking and the menu: which view is on screen, the split view being menu-only, and what the device remembers of it."""
import unittest

from geekmagic.core.views import SPLIT
from tests.support import AppTestCase, capture_screen


class SplitViewClicksTests(AppTestCase):
    def test_split_view_is_only_turned_on_from_the_menu_and_left_click_leaves_it(self):
        app = self.make()
        with capture_screen():
            self.assertEqual(app.view.key, "claude")
            app.toggle()  # the click only ever alternates Claude <-> Codex
            self.assertEqual((app.view.split, app.view.key), (False, "codex"))
            app.toggle_mode(SPLIT)
            self.assertEqual((app.view.split, app.view.key), (True, "split"))
            self.assertIn("dividida", app.tooltip())
            app.toggle()  # from the split view the click goes back to where you were
            self.assertEqual((app.view.split, app.view.key), (False, "codex"))
            app.toggle_mode(SPLIT)
            app.toggle_mode(SPLIT)  # the menu item toggles it off again
            self.assertEqual((app.view.split, app.view.key), (False, "codex"))
            app.toggle_mode(SPLIT)
            app.select("claude")  # picking a provider in the menu leaves it too
            self.assertEqual((app.view.split, app.view.key), (False, "claude"))

    def test_instant_switch_into_split_shows_its_last_image_only_while_active(self):
        app = self.make()
        app.screen.slots["split"] = "b"
        with capture_screen() as cap:
            app.screen.show_uploaded("split")  # not in split view: nothing happens
            self.assertEqual(cap.shown, [])
            app.view.split = True
            app.screen.show_uploaded("split")
        self.assertEqual(cap.shown, ["split-usage-b.gif"])

    def test_split_images_are_remembered_across_restarts_and_synced_with_the_device(self):
        app = self.make()
        app.screen.slots["split"] = "a"
        app.persistence.save()
        self.assertEqual(self.make().screen.slots.get("split"), "a")
        again = self.make()
        again.screen.device.files = set()  # the device lost its files
        again.screen.sync_with_device()
        self.assertNotIn("split", again.screen.slots)


if __name__ == "__main__":
    unittest.main()
