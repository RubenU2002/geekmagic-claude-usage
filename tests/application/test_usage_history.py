"""The recent readings of each usage window, from which the pace under each bar is measured."""
import json
import time
from dataclasses import replace
from datetime import datetime, timedelta

from geekmagic.application import config
from geekmagic.core.model import Usage, Window
from tests.support import AppTestCase, fake_fetchers


class UsageHistoryTests(AppTestCase):
    def reading(self, pct, reset_in_min=150, weekly=5.0, title="Claude") -> Usage:
        now = datetime.now().astimezone()
        return Usage(title=title, now=now, current=Window(pct, now + timedelta(minutes=reset_in_min)),
                     weekly=Window(weekly, now + timedelta(days=3)))

    def test_readings_are_recorded_when_they_change_and_not_repeated_needlessly(self):
        history = self.make().usage.history
        first = self.reading(10)
        history.record("claude", first)
        history.record("claude", replace(first))  # same percentage moments later
        self.assertEqual(len(history.points["claude/current"]), 1)
        history.record("claude", self.reading(12))
        self.assertEqual([p[1] for p in history.points["claude/current"]], [10, 12])
        self.assertEqual(len(history.points["claude/weekly"]), 1)

    def test_a_reset_drops_the_readings_of_the_previous_window(self):
        history = self.make().usage.history
        history.record("claude", self.reading(90, reset_in_min=10))
        history.record("claude", self.reading(3, reset_in_min=300))  # a new window began
        self.assertEqual([p[1] for p in history.points["claude/current"]], [3])

    def test_history_is_capped(self):
        history = self.make().usage.history
        for pct in range(config.HISTORY_MAX + 40):
            history.record("claude", self.reading(pct % 99))
        self.assertLessEqual(len(history.points["claude/current"]), config.HISTORY_MAX)

    def test_a_fetch_attaches_the_pace_and_a_recent_burst_raises_it(self):
        app = self.make()
        now = time.time()
        reset = datetime.now().astimezone() + timedelta(minutes=150)
        # 15 minutes ago it was at 30%, now 40%: a burst, faster than the 40/150 average since the window started
        app.usage.history.points["claude/current"] = [[now - 900, 30.0, reset.timestamp()]]
        with fake_fetchers(claude=lambda: self.reading(40)):
            read = app.usage.fetch("claude")
        rate = read.pace["current"].rate
        self.assertGreater(rate, 40 / 150)
        self.assertAlmostEqual(rate, 10 / 15, places=1)

    def test_history_survives_a_restart(self):
        app = self.make()
        app.usage.history.record("claude", self.reading(10))
        app.usage.history.record("claude", self.reading(14))
        app.persistence.save()
        self.assertEqual([p[1] for p in self.make().usage.history.points["claude/current"]], [10, 14])

    def test_a_garbled_history_is_ignored(self):
        self.store.path.write_text(json.dumps({"history": {"claude/current": [[1, 2], "x", [1, 2, 3]], "bad": 5}}))
        self.assertEqual(self.make().usage.history.points, {"claude/current": [[1, 2, 3]]})
