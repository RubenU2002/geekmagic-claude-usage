"""The views of both providers' activity (stats, projects and models, hours) and the split view: as menu modes, with their
own icons and slots, and how each one is drawn and uploaded."""
import json
import time
import unittest
from contextlib import contextmanager

from geekmagic.adapters.tray.icons import icon_for
from geekmagic.core.views import BREAKDOWN, HOURS, SPLIT, STATS
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


class StatsViewTests(AppTestCase):
    def test_the_stats_view_is_a_menu_mode_that_the_click_leaves(self):
        app = self.make()
        with capture_screen():
            app.toggle_mode(STATS)
            self.assertEqual((app.view.mode, app.view.key, app.view.split), ("stats", "stats", False))
            self.assertIn("estadísticas", app.tooltip())
            self.assertEqual(self.make().view.key, "stats", "remembered")
            app.toggle()
            self.assertEqual((app.view.mode, app.view.key), (None, "claude"))
            app.toggle_mode(STATS)
            app.toggle_mode(SPLIT)  # one view of both at a time
            self.assertEqual(app.view.mode, "split")
            app.toggle_mode(STATS)
            app.toggle_mode(STATS)
            self.assertEqual(app.view.mode, None)

    def test_the_stats_update_uploads_one_still_screen_with_both_providers_activity(self):
        app = self.make()
        app.view.mode = STATS
        with fake_fetchers(claude=lambda: usage(20, 5), codex=lambda: usage(10, 5, title="Codex")), \
                fake_counts(app) as calls, capture_screen() as cap:
            self.assertEqual(app.scheduler.update_panels("stats"), "ok")
            app.scheduler.update_panels("stats")  # the next cycle: the counts are not recomputed straight away
        key, panels, _ = cap.panels[0]
        self.assertEqual(key, "stats")
        self.assertEqual((cap.uploads[0], cap.shown[0]), ("stats-usage-a.gif", "stats-usage-a.gif"))
        self.assertEqual([p.title for p in panels], ["Claude", "Codex"])
        self.assertEqual([p.activity for p in panels], [ACTIVITY["claude"], ACTIVITY["codex"]],
                         "the same kind of numbers for both")
        self.assertEqual(sorted(calls), ["claude", "codex"], "each counted once, not once per cycle")

    def test_old_stats_images_are_redrawn_in_the_background_too(self):
        self.store.path.write_text(json.dumps({"slots": {"stats": "a"}, "render": "old", "view": "claude"}))
        app = self.make()
        app.usage.last_good = {p: (usage(title=t), time.monotonic()) for p, t in app.providers.titles.items()}
        with fake_counts(app), capture_screen() as cap:
            app.scheduler.refresh_outdated_views()
        self.assertEqual(cap.uploads, ["stats-usage-b.gif"])

    def test_split_uploads_carry_the_animation_choice(self):
        app = self.make()
        app.screen.animation = "coffee"
        with capture_screen() as cap:
            app.screen.upload("split", [usage()])
        self.assertEqual([animation for _, _, animation in cap.panels], ["coffee"])


class BreakdownAndHoursViewTests(AppTestCase):
    def test_the_two_new_views_are_menu_modes_like_the_others(self):
        app = self.make()
        with capture_screen():
            for mode, words in ((BREAKDOWN, "proyectos y modelos"), (HOURS, "horas pico")):
                app.toggle_mode(mode)
                self.assertEqual((app.view.mode, app.view.key), (mode, mode))
                self.assertIn(words, app.tooltip())
                self.assertEqual(self.make().view.key, mode, "remembered across restarts")
                app.toggle()  # the click leaves it
                self.assertEqual((app.view.mode, app.view.key), (None, "claude"))
            app.toggle_mode(HOURS)
            app.toggle_mode(BREAKDOWN)  # one view of both at a time
            self.assertEqual(app.view.mode, "breakdown")
            app.toggle_mode(BREAKDOWN)
            self.assertIsNone(app.view.mode)

    def test_each_new_view_has_its_own_icon_and_its_own_slots_on_the_device(self):
        app = self.make()
        keys = (STATS, BREAKDOWN, HOURS, SPLIT)
        for key in keys:
            self.assertEqual(icon_for(key, app.providers.titles).size, (64, 64))
        self.assertEqual(len({icon_for(k, app.providers.titles).tobytes() for k in keys}), 4)
        app.screen.slots.update(breakdown="a", hours="b")
        app.persistence.save()
        self.assertEqual({k: v for k, v in self.make().screen.slots.items() if k in ("breakdown", "hours")},
                         {"breakdown": "a", "hours": "b"})

    def test_each_view_is_asked_of_the_renderer_by_its_key_and_goes_to_its_own_slot_file(self):
        app = self.make()
        with capture_screen() as cap:
            for key in (STATS, BREAKDOWN, HOURS):
                app.screen.upload(key, [usage()])
        self.assertEqual([key for key, _, _ in cap.panels], ["stats", "breakdown", "hours"])
        self.assertEqual(cap.uploads, ["stats-usage-a.gif", "breakdown-usage-a.gif", "hours-usage-a.gif"])

    def test_the_activity_views_carry_each_providers_activity_and_count_it_once(self):
        app = self.make()
        activity = {p: {"24h": {"requests": 1, "sessions": 1}, "7d": {"requests": 1, "sessions": 1}, "days": [], "total": 1,
                        "projects": [], "models": [], "hours": [0] * 24} for p in app.providers}
        calls = []
        counters = {p: (lambda now, p=p: calls.append(p) or activity[p]) for p in app.providers}
        with fake_counts(app, **counters), \
                fake_fetchers(claude=lambda: usage(20, 5), codex=lambda: usage(10, 5, title="Codex")), \
                capture_screen() as cap:
            for mode in (BREAKDOWN, HOURS):
                app.view.mode = mode
                self.assertEqual(app.scheduler.update_panels(mode), "ok")
        self.assertEqual([key for key, _, _ in cap.panels], ["breakdown", "hours"])
        for _, panels, _ in cap.panels:
            self.assertEqual([p.activity for p in panels], [activity["claude"], activity["codex"]])
        self.assertEqual(sorted(calls), ["claude", "codex"], "both views share one count")


if __name__ == "__main__":
    unittest.main()
