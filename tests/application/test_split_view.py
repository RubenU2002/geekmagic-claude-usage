"""The view of both providers on one screen: what an update reads and uploads, and how a missing or stale provider shows."""
import time
import unittest
from unittest.mock import Mock

from geekmagic.application import config
from geekmagic.core.errors import UploadCancelled, UsageError
from tests.support import AppTestCase, capture_screen, failing, fake_fetchers, usage


class SplitUpdateTests(AppTestCase):
    def test_split_update_reads_both_providers_and_uploads_one_screen(self):
        app = self.make()
        app.view.split = True
        with capture_screen() as cap, \
                fake_fetchers(claude=lambda: usage(82, 5), codex=lambda: usage(10, 96, title="Codex")):
            self.assertEqual(app.scheduler.update_split(), "ok")
        (key, panels, _), = cap.panels
        self.assertEqual(key, "split")
        self.assertEqual([p.title for p in panels], ["Claude", "Codex"])
        self.assertEqual(cap.uploads, ["split-usage-a.gif"])
        self.assertEqual(cap.shown, ["split-usage-a.gif"])
        self.assertEqual(app.screen.slots["split"], "a")
        self.assertEqual(len(self.notes), 2, "alerts for both providers fire while the split view is on")

    def test_split_update_dims_only_the_provider_that_stopped_answering(self):
        app = self.make()
        app.view.split = True
        old = time.monotonic() - config.STALE_SECONDS - 5
        app.usage.last_good["codex"] = (usage(10, 20, title="Codex"), old)
        with capture_screen() as cap, \
                fake_fetchers(claude=lambda: usage(30, 5), codex=failing(UsageError("codex not logged in"))):
            app.scheduler.update_split()
        _, (claude, codex), _ = cap.panels[0]
        self.assertFalse(claude.stale)
        self.assertTrue(codex.stale)
        self.assertEqual(codex.current.pct, 10)

    def test_split_update_keeps_working_with_one_provider_missing(self):
        app = self.make()
        app.view.split = True
        with capture_screen() as cap, \
                fake_fetchers(claude=lambda: usage(30, 5), codex=failing(UsageError("codex not installed"))):
            self.assertEqual(app.scheduler.update_split(), "ok")
            _, panels, _ = cap.panels[0]
            self.assertEqual([x.title for x in panels], ["Claude", "Codex"], "the one that failed keeps its panel...")
            self.assertIsNone(panels[1].current.pct, "...with dashes instead of numbers")
            app.usage.last_good.clear()
            with fake_fetchers(claude=failing(UsageError("claude not installed"))):
                self.assertEqual(app.scheduler.update_split(), "error")  # nothing to show at all

    def test_a_click_cancels_an_upload_of_the_split_view_too(self):
        app = self.make()
        app.view.split = True
        app.screen.device.upload = Mock(side_effect=UploadCancelled())
        with capture_screen():
            self.assertEqual(app.screen.deliver("split", [usage()]), "cancelled")
        self.assertNotIn("split", app.screen.slots)


if __name__ == "__main__":
    unittest.main()
