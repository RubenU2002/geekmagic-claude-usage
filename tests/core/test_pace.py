"""Pace projection: where a window will end up, and when it would run out."""
import unittest
from datetime import datetime, timedelta

from geekmagic.core import pace
from geekmagic.core.model import Usage, Window

NOW = datetime(2026, 10, 2, 14, 57).astimezone()


def usage(current=None, current_left_min=None, weekly=None, weekly_left_h=None, current_window_min=None, **extra):
    return Usage(
        title="Claude", now=NOW,
        current=Window(current, NOW + timedelta(minutes=current_left_min) if current_left_min is not None else None,
                       current_window_min),
        weekly=Window(weekly, NOW + timedelta(hours=weekly_left_h) if weekly_left_h is not None else None), **extra)


class ProjectTests(unittest.TestCase):
    def test_steady_pace_projects_to_the_end_of_the_window(self):
        # 3.5 h into a 5 h session at 62%: 62 / 210 min per minute, 90 minutes to go
        p = pace.project(62, NOW + timedelta(minutes=90), NOW, 300)
        self.assertAlmostEqual(p.end, 62 + 62 / 210 * 90, places=3)
        self.assertFalse(p.hits_limit)
        self.assertIsNone(p.minutes_to_limit)

    def test_heading_past_100_says_when(self):
        # 2 h in at 80%: 40%/h, so 20 points left last 30 minutes
        p = pace.project(80, NOW + timedelta(minutes=180), NOW, 300)
        self.assertTrue(p.hits_limit)
        self.assertAlmostEqual(p.minutes_to_limit, 30, places=3)

    def test_too_early_or_too_little_used_gives_nothing(self):
        self.assertIsNone(pace.project(30, NOW + timedelta(minutes=290), NOW, 300), "10 minutes into the window")
        self.assertIsNone(pace.project(1.0, NOW + timedelta(minutes=100), NOW, 300), "almost nothing used")
        self.assertIsNone(pace.project(None, NOW + timedelta(minutes=100), NOW, 300))
        self.assertIsNone(pace.project(50, None, NOW, 300))

    def test_a_window_that_already_ended_or_is_full_gives_nothing_alarming(self):
        self.assertIsNone(pace.project(50, NOW - timedelta(minutes=1), NOW, 300))
        full = pace.project(100, NOW + timedelta(minutes=60), NOW, 300)
        self.assertFalse(full.hits_limit, "already at the limit: nothing left to predict")

    def test_a_recent_burst_beats_the_average(self):
        calm = pace.project(40, NOW + timedelta(minutes=150), NOW, 300)  # 150 min in: 40/150 per min
        burst = pace.project(40, NOW + timedelta(minutes=150), NOW, 300, recent_rate=0.8)
        self.assertGreater(burst.rate, calm.rate)
        self.assertEqual(burst.rate, 0.8)
        self.assertTrue(burst.hits_limit)

    def test_a_slow_recent_rate_does_not_hide_a_heavy_start(self):
        p = pace.project(40, NOW + timedelta(minutes=150), NOW, 300, recent_rate=0.01)
        self.assertAlmostEqual(p.rate, 40 / 150)


class RecentRateTests(unittest.TestCase):
    RESET = 1_800_000_000.0

    def history(self, *points, reset=None):
        return [[t, p, reset if reset is not None else self.RESET] for t, p in points]

    def test_rate_over_the_lookback(self):
        now = 1_000_000.0
        h = self.history((now - 1500, 10), (now - 900, 14), (now, 20))  # +10 points over 25 minutes
        self.assertAlmostEqual(pace.recent_rate(h, self.RESET, now, 30), 10 / 25)

    def test_needs_enough_history(self):
        now = 1_000_000.0
        self.assertIsNone(pace.recent_rate([], self.RESET, now, 30))
        self.assertIsNone(pace.recent_rate(self.history((now, 20)), self.RESET, now, 30), "a single reading")
        self.assertIsNone(pace.recent_rate(self.history((now - 120, 18), (now, 20)), self.RESET, now, 30), "under 5 minutes")

    def test_a_short_burst_is_not_stretched_over_the_whole_look_back(self):
        """The real case: +3 points over 45 minutes must not become a weekly pace worth days of "Full in 19h"."""
        now = 1_000_000.0
        burst = self.history((now - 1200, 17), (now, 20))  # 20 minutes of history
        self.assertIsNone(pace.recent_rate(burst, self.RESET, now, pace.LOOKBACK_MIN["weekly"]), "a day's look-back needs 6 h of it")
        self.assertAlmostEqual(pace.recent_rate(burst, self.RESET, now, pace.LOOKBACK_MIN["current"]), 3 / 20,
                               msg="but it is plenty for a 30-minute look-back")
        day = self.history((now - 8 * 3600, 12), (now - 4 * 3600, 16), (now, 20))  # 8 hours: enough
        self.assertAlmostEqual(pace.recent_rate(day, self.RESET, now, pace.LOOKBACK_MIN["weekly"]), 8 / 480)

    def test_the_weekly_look_back_is_a_day_and_the_session_one_half_an_hour(self):
        self.assertEqual(pace.LOOKBACK_MIN, {"current": 30, "weekly": 1440})

    def test_flat_or_falling_is_no_rate(self):
        now = 1_000_000.0
        self.assertIsNone(pace.recent_rate(self.history((now - 900, 20), (now, 20)), self.RESET, now, 30))
        self.assertIsNone(pace.recent_rate(self.history((now - 900, 50), (now, 3)), self.RESET, now, 30))

    def test_readings_of_an_earlier_window_are_ignored(self):
        now = 1_000_000.0
        old = [[now - 900, 90, self.RESET - 18000], [now - 800, 95, self.RESET - 18000]]  # before the last reset
        self.assertIsNone(pace.recent_rate(old + [[now, 4, self.RESET]], self.RESET, now, 30))

    def test_readings_older_than_the_lookback_are_ignored(self):
        now = 1_000_000.0
        h = self.history((now - 7200, 1), (now - 600, 10), (now, 12))
        self.assertAlmostEqual(pace.recent_rate(h, self.RESET, now, 30), 2 / 10)


class DescribeTests(unittest.TestCase):
    def test_formats(self):
        self.assertEqual([pace.format_minutes(m) for m in (0, 7, 59, 60, 130, 1439, 1440, 3 * 1440 + 4 * 60 + 30)],
                         ["0m", "7m", "59m", "1h 0m", "2h 10m", "23h 59m", "1d 0h", "3d 4h"])

    def test_texts_and_severity(self):
        calm = pace.project(62, NOW + timedelta(minutes=90), NOW, 300)
        self.assertEqual(pace.describe(calm), ("~89% at reset", "ok"))
        soon = pace.project(80, NOW + timedelta(minutes=180), NOW, 300)
        self.assertEqual(pace.describe(soon), ("Full in 30m", "crit"))  # under an hour
        later = pace.project(30, NOW + timedelta(minutes=240), NOW, 300)
        self.assertEqual(pace.describe(later), ("Full in 2h 20m", "warn"))
        self.assertIsNone(pace.describe(None))


class AnnotateTests(unittest.TestCase):
    def test_adds_a_projection_per_window_using_each_windows_length(self):
        u = pace.annotate(usage(current=62, current_left_min=90, weekly=95, weekly_left_h=24))
        self.assertFalse(u.pace["current"].hits_limit)
        self.assertTrue(u.pace["weekly"].hits_limit, "95% with a day of 7 left runs out")
        calm = pace.annotate(usage(weekly=70, weekly_left_h=24)).pace["weekly"]
        self.assertFalse(calm.hits_limit, "70% with a day of 7 left is fine")
        self.assertAlmostEqual(calm.end, 70 + 70 / 144 * 24, places=3)  # judged on a 7-day window, not a 5-hour one

    def test_codex_reported_window_lengths_win(self):
        a = pace.annotate(usage(current=62, current_left_min=90)).pace["current"]
        b = pace.annotate(usage(current=62, current_left_min=90, current_window_min=600)).pace["current"]
        self.assertNotEqual(a.rate, b.rate)

    def test_recent_rates_are_used(self):
        u = pace.annotate(usage(current=40, current_left_min=150), recent={"current": 0.9})
        self.assertEqual(u.pace["current"].rate, 0.9)

    def test_missing_windows_are_none(self):
        self.assertEqual(pace.annotate(usage()).pace, {"current": None, "weekly": None})


if __name__ == "__main__":
    unittest.main()
