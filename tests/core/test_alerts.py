"""Thresholds: bar colour states and when notifications fire."""
import unittest

from geekmagic.core import alerts


class BarStateTests(unittest.TestCase):
    def test_states(self):
        self.assertEqual(alerts.bar_state(None), "ok")
        self.assertEqual(alerts.bar_state(0), "ok")
        self.assertEqual(alerts.bar_state(69.9), "ok")
        self.assertEqual(alerts.bar_state(70), "warn")
        self.assertEqual(alerts.bar_state(89.9), "warn")
        self.assertEqual(alerts.bar_state(90), "crit")
        self.assertEqual(alerts.bar_state(100), "crit")


class EvaluateTests(unittest.TestCase):
    def test_level(self):
        self.assertEqual([alerts.level(p) for p in (None, 0, 79, 80, 94, 95, 100)], [0, 0, 0, 1, 1, 2, 2])

    def test_no_event_below_threshold_or_without_reading(self):
        self.assertEqual(alerts.evaluate(0, None, 50), (0, None))
        self.assertEqual(alerts.evaluate(1, 85, None), (1, None))

    def test_rise_notifies_once_per_threshold(self):
        self.assertEqual(alerts.evaluate(0, 50, 82), (1, ("rise", 80)))
        self.assertEqual(alerts.evaluate(1, 82, 90), (1, None))  # still the same level: nothing new
        self.assertEqual(alerts.evaluate(1, 90, 96), (2, ("rise", 95)))

    def test_first_reading_already_high_raises_the_highest_threshold_only(self):
        self.assertEqual(alerts.evaluate(0, None, 96), (2, ("rise", 95)))

    def test_reset_after_an_alert(self):
        self.assertEqual(alerts.evaluate(2, 96, 3), (0, ("reset", None)))
        self.assertEqual(alerts.evaluate(1, 82, 50), (0, ("reset", None)))  # a fall of 30+ points

    def test_no_reset_message_if_nothing_had_alerted(self):
        self.assertEqual(alerts.evaluate(0, 60, 2), (0, None))

    def test_hovering_around_a_threshold_does_not_renotify(self):
        level, event = alerts.evaluate(1, 82, 78)  # small dip: stays armed-off
        self.assertEqual((level, event), (1, None))
        self.assertEqual(alerts.evaluate(level, 78, 81), (1, None))  # back above 80: no second notification
        self.assertEqual(alerts.evaluate(1, 81, 74), (0, None))  # a real drop re-arms it, silently
        self.assertEqual(alerts.evaluate(0, 74, 80), (1, ("rise", 80)))  # ...so it can fire again


if __name__ == "__main__":
    unittest.main()
