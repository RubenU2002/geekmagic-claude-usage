"""Counting each provider's local activity (for the stats views): when, in a thread, and what happens when it fails."""
import threading
import time
import unittest
from contextlib import contextmanager
from unittest.mock import patch

from geekmagic.application import config
from tests.support import AppTestCase, capture_screen, fake_fetchers, usage

ACTIVITY = {
    "claude": {"24h": {"requests": 422, "sessions": 4}, "7d": {"requests": 1584, "sessions": 13},
               "days": [{"date": f"2026-09-{26 + i}", "requests": i} for i in range(7)]},
    "codex": {"24h": {"requests": 26, "sessions": 1}, "7d": {"requests": 1521, "sessions": 17},
              "days": [{"date": f"2026-09-{26 + i}", "requests": 10 * i} for i in range(7)]},
}


@contextmanager
def fake_counts(app, **overrides):
    """Replace how each provider's local activity is counted; yields the log of which providers were counted."""
    calls = []
    counters = {p: (lambda now, p=p: calls.append(p) or ACTIVITY[p]) for p in app.providers}
    counters.update(overrides)
    for key, counter in counters.items():
        app.providers[key].stats = counter
    yield calls


class ActivityTests(AppTestCase):
    def test_the_counts_come_back_after_their_interval(self):
        app = self.make()
        with fake_counts(app) as calls:
            app.activity.refresh()
            app.activity.refresh()
            self.assertEqual(len(calls), 2)
            app.activity.counted_at -= config.LOCAL_STATS_EVERY + 1
            app.activity.refresh()
        self.assertEqual(len(calls), 4)

    def test_a_failing_count_does_not_break_the_stats_view(self):
        app = self.make()
        app.view.mode = "stats"

        def broken(now):
            raise OSError("disk")

        with fake_counts(app, codex=broken), \
                fake_fetchers(claude=lambda: usage(20, 5), codex=lambda: usage(10, 5, title="Codex")), capture_screen() as cap:
            self.assertEqual(app.scheduler.update_panels("stats"), "ok")
        _, (claude, codex), _ = cap.panels[0]
        self.assertEqual(claude.activity, ACTIVITY["claude"])
        self.assertIsNone(codex.activity, "no numbers yet: the screen says it's counting")

    def test_a_provider_without_logs_is_marked_as_having_none(self):
        app = self.make()
        with fake_counts(app, claude=lambda now: None):
            app.activity.refresh()
        self.assertEqual(app.activity.stats["claude"], {})
        self.assertEqual(app.activity.stats["codex"], ACTIVITY["codex"])

    def test_the_count_runs_in_a_thread_and_wakes_the_loop_when_done(self):
        app = self.make()
        gate = threading.Event()

        def slow(now):
            gate.wait(2)
            return {"24h": {"requests": 1, "sessions": 1}}

        with patch.object(config, "BACKGROUND_STATS", True), fake_counts(app, claude=slow, codex=slow):
            app.wake.clear()
            app.activity.refresh()
            self.assertTrue(app.activity.counting, "counting in the background")
            self.assertEqual(app.activity.stats, {}, "the screens say 'Counting...' meanwhile")
            app.activity.refresh()  # a second request while one is running does nothing
            gate.set()
            for _ in range(100):
                if not app.activity.counting:
                    break
                time.sleep(0.02)
        self.assertFalse(app.activity.counting)
        self.assertEqual(sorted(app.activity.stats), ["claude", "codex"])
        self.assertTrue(app.wake.is_set(), "the loop is woken to redraw")

    def test_a_failed_count_is_retried_sooner_than_a_good_one_is_refreshed(self):
        app = self.make()
        calls = []

        def broken(now):
            calls.append(1)
            raise OSError("disk")

        with fake_counts(app, claude=lambda now: {"24h": {}}, codex=broken):
            app.activity.refresh()
            app.activity.refresh()
            self.assertEqual(len(calls), 1, "not again straight away")
            app.activity.counted_at -= config.LOCAL_STATS_RETRY + 1
            app.activity.refresh()
            self.assertEqual(len(calls), 2, "incomplete: retried after a minute, not after ten")


if __name__ == "__main__":
    unittest.main()
