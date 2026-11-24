"""Activity stats: the counting is the same for every provider, whatever its logs looked like (events in, numbers out)."""
import time
import unittest
from datetime import datetime, timedelta

from geekmagic.core import activity as s


def ago(**delta) -> float:
    return (datetime.now().astimezone() - timedelta(**delta)).timestamp()


def local(days_ago: int, hour: int, minute: int = 0) -> float:
    """A local wall-clock time `days_ago` days back, so a test can place events on specific calendar days."""
    day = datetime.now().date() - timedelta(days=days_ago)
    return datetime(day.year, day.month, day.day, hour, minute).astimezone().timestamp()


def event(when: float, session="A", mid=None, project="Alpha", model="m1"):
    """One model call, as a provider's log reader produces it: (epoch, session, unique id or None, project, model)."""
    return (when, session, mid, project, model)


def summary(events):
    return s.summarize(events, time.time())


class CountTests(unittest.TestCase):
    def test_counts_requests_and_sessions_per_window(self):
        stats = summary([event(ago(hours=1), "A", "1"), event(ago(hours=3), "A", "2"), event(ago(hours=30), "A", "3"),
                         event(ago(days=3), "B", "4"), event(ago(days=3, hours=1), "B", "5")])
        self.assertEqual(stats["24h"], {"requests": 2, "sessions": 1})
        self.assertEqual(stats["7d"], {"requests": 5, "sessions": 2})

    def test_a_request_seen_twice_counts_once(self):
        when = ago(hours=1)
        stats = summary([event(when, "A", "same"), event(when, "A", "same"), event(when, "A", "other")])
        self.assertEqual(stats["24h"]["requests"], 2)

    def test_the_same_id_in_another_session_is_another_request(self):
        stats = summary([event(ago(hours=1), "A", "1"), event(ago(hours=1), "B", "1")])
        self.assertEqual(stats["24h"], {"requests": 2, "sessions": 2})

    def test_requests_without_an_id_are_each_counted(self):
        when = ago(hours=1)
        self.assertEqual(summary([event(when), event(when), event(when)])["24h"]["requests"], 3)

    def test_every_request_of_one_session_is_one_session(self):
        stats = summary([event(ago(hours=2), "P", "1"), event(ago(hours=2), "P", "2"), event(ago(hours=1), "P", "3")])
        self.assertEqual(stats["24h"], {"requests": 3, "sessions": 1})

    def test_no_activity_at_all_is_zeroes_not_nothing(self):
        stats = summary([])
        self.assertEqual(stats["24h"], {"requests": 0, "sessions": 0})
        self.assertEqual([d["requests"] for d in stats["days"]], [0] * 7)
        self.assertEqual(stats["hours"], [0] * 24)
        self.assertEqual((stats["projects"], stats["models"], stats["total"]), ([], [], 0))


class DaysTests(unittest.TestCase):
    def test_the_chart_covers_today_and_the_six_days_before_oldest_first(self):
        days = summary([event(ago(hours=1))])["days"]
        self.assertEqual(len(days), 7)
        self.assertEqual(days[-1]["date"], datetime.now().date().isoformat())
        self.assertEqual(days[0]["date"], (datetime.now().date() - timedelta(days=6)).isoformat())
        self.assertEqual([d["date"] for d in days], sorted(d["date"] for d in days))

    def test_requests_land_on_their_local_calendar_day(self):
        whens = [local(0, 0, 5), local(0, 10), local(1, 23, 30), local(1, 8), local(3, 12), local(6, 1)]
        by_date = {d["date"]: d["requests"] for d in summary([event(w) for w in whens])["days"]}
        today = datetime.now().date()
        self.assertEqual(by_date[today.isoformat()], 2)
        self.assertEqual(by_date[(today - timedelta(days=1)).isoformat()], 2)
        self.assertEqual(by_date[(today - timedelta(days=3)).isoformat()], 1)
        self.assertEqual(by_date[(today - timedelta(days=6)).isoformat()], 1)
        self.assertEqual(by_date[(today - timedelta(days=2)).isoformat()], 0, "days without activity are there, at zero")

    def test_anything_older_than_the_chart_is_left_out(self):
        stats = summary([event(local(7, 12)), event(local(20, 12)), event(ago(hours=1))])
        self.assertEqual(sum(d["requests"] for d in stats["days"]), 1)


class BreakdownTests(unittest.TestCase):
    def test_requests_are_split_by_project_and_model(self):
        stats = summary([
            event(ago(hours=1), "A", "1", "Alpha", "Opus 5.5"),
            event(ago(hours=2), "A", "2", "Alpha", "Opus 5.5"),
            event(ago(hours=3), "A", "3", "Alpha", "Sonnet 5"),
            event(ago(hours=4), "B", "4", "Beta", "Sonnet 5"),
            event(ago(days=3), "B", "5", "Beta", "Haiku 4.5"),
            event(ago(days=10), "C", "6", "Old", "Opus 5.5"),  # outside the week
        ])
        self.assertEqual(stats["total"], 5)
        self.assertEqual([(p["name"], p["requests"]) for p in stats["projects"]], [("Alpha", 3), ("Beta", 2)])
        self.assertEqual({m["name"]: m["requests"] for m in stats["models"]}, {"Opus 5.5": 2, "Sonnet 5": 2, "Haiku 4.5": 1})

    def test_the_busiest_comes_first_and_only_the_top_are_kept(self):
        projects = summary([event(ago(minutes=i), "A", str(i), f"project{i % 9}") for i in range(60)])["projects"]
        self.assertEqual(len(projects), s.TOP)
        self.assertEqual([p["requests"] for p in projects], sorted((p["requests"] for p in projects), reverse=True))

    def test_shares_are_fractions_of_the_total(self):
        items = [{"name": "a", "requests": 50}, {"name": "b", "requests": 30}, {"name": "c", "requests": 10}, {"name": "d", "requests": 5}]
        self.assertEqual(s.shares(items, 100), [("a", 0.5), ("b", 0.3), ("c", 0.1)])
        self.assertEqual(s.shares(items, 100, limit=1), [("a", 0.5)])
        self.assertEqual(s.shares(items, 0), [], "no total, no shares")


class HoursTests(unittest.TestCase):
    def test_requests_are_counted_per_local_hour_of_the_day(self):
        whens = [local(1, 9, 5), local(1, 9, 50), local(2, 9, 0), local(3, 14, 30), local(5, 0, 10), local(5, 23, 59)]
        hours = summary([event(w) for w in whens])["hours"]
        self.assertEqual(len(hours), 24)
        self.assertEqual((hours[9], hours[14], hours[0], hours[23], hours[3]), (3, 1, 1, 1, 0))

    def test_it_looks_back_four_weeks_not_just_the_chart_week(self):
        hours = summary([event(local(20, 10)), event(local(27, 10)), event(local(29, 10))])["hours"]
        self.assertEqual(hours[10], 2, "20 and 27 days ago are inside the four weeks, 29 isn't")

    def test_the_chart_week_is_unaffected_by_the_longer_look_back(self):
        stats = summary([event(local(20, 10)), event(local(1, 10))])
        self.assertEqual(sum(d["requests"] for d in stats["days"]), 1)

    def test_busiest_stretch(self):
        hours = [0] * 24
        hours[14], hours[15], hours[9] = 40, 30, 50
        self.assertEqual(s.busiest_hours(hours), (14, 16), "two consecutive hours beat one tall one")
        self.assertIsNone(s.busiest_hours([0] * 24))
        late = [0] * 24
        late[23], late[0] = 10, 12
        self.assertEqual(s.busiest_hours(late), (23, 1), "wraps past midnight")

    def test_busiest_stretch_ties_go_to_the_one_that_starts_on_a_busy_hour_then_the_earlier(self):
        hours = [0] * 24
        hours[6], hours[18] = 5, 5
        self.assertEqual(s.busiest_hours(hours)[0], 6, "not 5-7, which would start on an empty hour")
        flat = [3] * 24
        self.assertEqual(s.busiest_hours(flat)[0], 0, "all equal: the earliest")

    def test_clock_text(self):
        self.assertEqual([s.hour12(h) for h in (0, 1, 11, 12, 13, 23)], ["12 AM", "1 AM", "11 AM", "12 PM", "1 PM", "11 PM"])
        self.assertEqual(s.hours_text((14, 16)), "2-4 PM")
        self.assertEqual(s.hours_text((11, 13)), "11 AM-1 PM")
        self.assertEqual(s.hours_text((23, 1)), "11 PM-1 AM")
        self.assertEqual(s.hours_text((8, 10)), "8-10 AM")
        self.assertEqual(s.hours_text((12, 14)), "12-2 PM")


class LabelTests(unittest.TestCase):
    def test_project_is_the_folder_name_whatever_the_slashes_or_drive_case(self):
        for cwd in ("C:\\Work\\Acme App", "c:\\Work\\Acme App", "/home/me/work/Acme App", "/home/me/work/Acme App/"):
            self.assertEqual(s.project_label(cwd), "Acme App", cwd)

    def test_generic_folders_are_shown_with_their_parent(self):
        self.assertEqual(s.project_label("C:\\Work\\Storefront\\frontend"), "Storefront/frontend")
        self.assertEqual(s.project_label("/srv/shop/SRC"), "shop/SRC")
        self.assertEqual(s.project_label("C:\\src"), "src", "nothing but a drive above it")
        self.assertEqual(s.project_label("/src"), "src")

    def test_unknown_when_there_is_no_folder(self):
        self.assertEqual([s.project_label(c) for c in (None, "", "/")], ["(unknown)"] * 3)


class TextTests(unittest.TestCase):
    def test_counts_text_with_singulars_and_separators(self):
        self.assertEqual(s.counts_text("24h", {"requests": 375, "sessions": 4}), "24h · 375 requests · 4 sessions")
        self.assertEqual(s.counts_text("7d", {"requests": 1560, "sessions": 13}), "7d · 1,560 requests · 13 sessions")
        self.assertEqual(s.counts_text("24h", {"requests": 1, "sessions": 1}), "24h · 1 request · 1 session")

    def test_weekday_names_are_english_whatever_the_system_language(self):
        self.assertEqual([s.weekday(f"2026-10-{d:02d}") for d in range(5, 12)], ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"])


if __name__ == "__main__":
    unittest.main()
