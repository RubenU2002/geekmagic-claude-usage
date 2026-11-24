"""Coming back to the view you left: after a restart, after a damaged state file, and when the device returns."""
import json
import unittest
from unittest.mock import Mock

from geekmagic.core.views import SPLIT
from tests.support import AppTestCase, FakeDisplay, FakeRenderer, capture_screen, usage


class ViewRestoredTests(AppTestCase):
    def test_the_view_you_left_is_restored_after_a_restart(self):
        with capture_screen():
            first = self.make()
            self.assertEqual((first.view.split, first.view.provider), (False, "claude"))  # first ever run
            first.select("codex")
            again = self.make()
            self.assertEqual((again.view.split, again.view.provider, again.view.key), (False, "codex", "codex"))
            self.assertIn("Codex", again.tooltip())

            again.toggle_mode(SPLIT)
            back = self.make()
            self.assertEqual((back.view.split, back.view.key), (True, "split"))
            self.assertEqual(back.view.provider, "codex")  # what leaving the split view returns to
            self.assertIn("dividida", back.tooltip())
            back.toggle()
            self.assertEqual((back.view.split, back.view.provider), (False, "codex"))

            back.select("claude")
            self.assertEqual(self.make().view.key, "claude")

    def test_provider_argument_is_only_the_first_runs_choice(self):
        first_run = self.make(provider="codex")  # nothing remembered yet
        self.assertEqual(first_run.view.provider, "codex")
        with capture_screen():
            self.make().select("claude")
        self.assertEqual(self.make(provider="codex").view.provider, "claude")  # the remembered one wins

    def test_startup_puts_the_remembered_view_back_on_screen(self):
        for view, expected in (("split", "split-usage-b.gif"), ("codex", "codex-usage-b.gif")):
            self.store.path.write_text(json.dumps({
                "slots": {"claude": "a", "codex": "b", "split": "b"}, "view": view, "provider": "codex",
                "render": FakeRenderer.version,
            }))

            def display_with_its_files(address):  # the device still has every image it was left with
                display = FakeDisplay(address)
                display.files = {"claude-usage-a.gif", "codex-usage-b.gif", "split-usage-b.gif"}
                return display

            app = self.make(display_factory=display_with_its_files)
            app.stop.set()  # run just the start-up part of the worker
            with capture_screen() as cap:
                app.scheduler.run()  # (the locator answers every probe by default)
            self.assertEqual(cap.shown, [expected], view)

    def test_a_garbled_state_file_falls_back_to_the_defaults(self):
        self.store.path.write_text(json.dumps({"view": "nonsense", "provider": 7, "slots": "x"}))
        app = self.make()
        self.assertEqual((app.view.split, app.view.provider), (False, "claude"))


class DeviceComesBackTests(AppTestCase):
    def test_when_the_device_comes_back_the_remembered_view_is_pinned_before_the_upload(self):
        app = self.make()
        app.view.provider, app.screen.slots["codex"] = "codex", "b"
        app.screen.offline = True
        with capture_screen():
            outcome = app.screen.deliver("codex", usage(title="Codex"))
        self.assertEqual(outcome, "ok")
        calls = app.screen.device.calls
        self.assertEqual(calls[0], ("show", "codex-usage-b.gif"))  # the view is back on screen right away
        self.assertIn(("upload", "codex-usage-a.gif"), calls)  # then the fresh image goes into the spare slot
        self.assertFalse(app.screen.offline)

    def test_while_the_device_is_still_off_one_small_request_is_all_it_costs(self):
        app = self.make()
        app.screen.slots["claude"] = "a"
        app.screen.offline = True
        app.screen.device.show_image = Mock(side_effect=OSError("timed out"))
        with capture_screen() as cap:
            self.assertEqual(app.screen.deliver("claude", usage()), "offline")
        self.assertEqual(cap.uploads, [])
        self.assertTrue(app.screen.offline)


if __name__ == "__main__":
    unittest.main()
