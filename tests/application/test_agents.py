"""Is Claude / Codex working, waiting for you, or done: what the screen and the tooltip say, and what gets notified."""
import time
import unittest

from tests.support import AppTestCase, usage


class AgentTests(AppTestCase):
    def state(self, working, since=None, project="Acme App"):
        return {"working": working, "since": since, "last": time.time(), "project": project}

    def waiting(self, reason="question", project="Acme App", since=None):
        return {**self.state(True, since=since or time.time() - 100, project=project), "waiting": True, "waiting_for": reason}

    # --- agents working: indicator and "finished" notifications ------------------------------------------------

    def test_a_run_starting_is_shown_at_once_and_flags_the_redraw(self):
        app = self.make()
        app.wake.clear()
        app.agents.update("claude", self.state(True, since=time.time() - 30))
        self.assertTrue(app.agents.working("claude"))
        self.assertFalse(app.agents.working("codex"))
        self.assertTrue(app.agents.dirty)
        self.assertTrue(app.wake.is_set())
        self.assertIn("Claude trabajando", app.tooltip())
        self.assertEqual(self.notes, [], "starting isn't worth a notification")

    def test_a_run_is_only_over_after_two_quiet_polls_in_a_row(self):
        app = self.make()
        app.agents.update("claude", self.state(True, since=time.time() - 300))
        app.agents.dirty = False
        app.agents.update("claude", self.state(False))
        self.assertTrue(app.agents.working("claude"), "one quiet poll: probably just a gap between steps")
        self.assertFalse(app.agents.dirty)
        app.agents.update("claude", self.state(True, since=time.time() - 300))  # it picked up again
        app.agents.update("claude", self.state(False))
        self.assertTrue(app.agents.working("claude"), "the quiet streak starts over")
        app.agents.update("claude", self.state(False))
        self.assertFalse(app.agents.working("claude"))
        self.assertTrue(app.agents.dirty)

    def test_a_long_run_finishing_notifies_with_how_long_and_where(self):
        app = self.make()
        app.agents.update("codex", self.state(True, since=time.time() - 250, project="Storefront"))
        app.agents.update("codex", self.state(False, project="Storefront"))
        app.agents.update("codex", self.state(False, project="Storefront"))
        title, message = self.notes[-1]
        self.assertEqual(title, "Codex · Storefront", "the project is in the title, the first thing you read")
        self.assertEqual(message, "Terminó (trabajó 4 min)")

    def test_short_runs_finish_without_a_notification(self):
        app = self.make()
        app.agents.update("claude", self.state(True, since=time.time() - 20))
        for _ in range(2):
            app.agents.update("claude", self.state(False))
        self.assertEqual(self.notes, [])
        self.assertFalse(app.agents.working("claude"))

    def test_the_finished_notification_can_be_turned_off_and_is_remembered(self):
        app = self.make()
        app.toggle_agent_events()
        self.assertFalse(app.notifications.agent_events)
        app.agents.update("claude", self.state(True, since=time.time() - 400))
        for _ in range(2):
            app.agents.update("claude", self.state(False))
        self.assertEqual(self.notes, [])
        self.assertTrue(app.agents.dirty, "the screen still updates")
        self.assertFalse(self.make().notifications.agent_events)

    def test_a_run_already_under_way_when_the_app_starts_is_timed_from_the_log(self):
        app = self.make()
        app.agents.update("claude", self.state(True, since=time.time() - 900))  # started 15 minutes before we looked
        for _ in range(2):
            app.agents.update("claude", self.state(False))
        self.assertIn("15 min", self.notes[-1][1])

    # --- an agent waiting for you ------------------------------------------------------------------------------

    def test_waiting_is_only_believed_after_two_polls_in_a_row(self):
        app = self.make()
        app.agents.update("claude", self.waiting())
        self.assertFalse(app.agents.waiting("claude"), "one poll could be a flicker")
        self.assertEqual(self.notes, [])
        self.assertTrue(app.agents.working("claude"))
        app.agents.update("claude", self.state(True, since=time.time() - 100))  # it moved on: the streak starts over
        app.agents.update("claude", self.waiting())
        self.assertFalse(app.agents.waiting("claude"))
        app.agents.update("claude", self.waiting())
        self.assertTrue(app.agents.waiting("claude"))
        self.assertEqual(len(self.notes), 1)

    def test_the_notification_says_what_it_is_waiting_for_and_where(self):
        cases = (("question", "Te hizo una pregunta"), ("plan", "Espera que apruebes su plan"),
                 ("approval (Edit)", "Espera tu aprobación (Edit)"), ("approval", "Espera tu aprobación"),
                 ("something new", "Espera tu respuesta"))
        for reason, expected in cases:
            app = self.make()
            self.notes.clear()
            for _ in range(2):
                app.agents.update("claude", self.waiting(reason))
            self.assertEqual(self.notes[-1], ("Claude · Acme App", expected), "the project is in the title")
        app = self.make()
        self.notes.clear()
        for _ in range(2):
            app.agents.update("codex", self.waiting("question", project=None))
        self.assertEqual(self.notes[-1], ("Codex", "Te hizo una pregunta"), "without a project, just the provider")

    def test_waiting_flags_a_redraw_and_shows_in_the_tooltip(self):
        app = self.make()
        app.agents.dirty = False
        for _ in range(2):
            app.agents.update("codex", self.waiting("approval"))
        self.assertTrue(app.agents.dirty)
        self.assertIn("Codex te espera", app.tooltip())
        self.assertNotIn("trabajando", app.tooltip())

    def test_answering_clears_it_and_redraws_but_it_keeps_working(self):
        app = self.make()
        for _ in range(2):
            app.agents.update("claude", self.waiting())
        app.agents.dirty = False
        self.notes.clear()
        app.agents.update("claude", self.state(True, since=time.time() - 100))
        self.assertFalse(app.agents.waiting("claude"))
        self.assertTrue(app.agents.working("claude"))
        self.assertTrue(app.agents.dirty)
        self.assertEqual(self.notes, [], "no notification for being answered")
        self.assertIn("Claude trabajando", app.tooltip())

    def test_one_notification_per_wait_not_one_per_poll(self):
        app = self.make()
        for _ in range(10):
            app.agents.update("claude", self.waiting())
        self.assertEqual(len(self.notes), 1)

    def test_it_can_be_silenced_with_the_same_option_as_the_finished_notification(self):
        app = self.make()
        app.notifications.agent_events = False
        for _ in range(3):
            app.agents.update("claude", self.waiting())
        self.assertEqual(self.notes, [])
        self.assertTrue(app.agents.waiting("claude"), "the screen still shows it")

    def test_waiting_then_finishing_still_reports_the_whole_run(self):
        app = self.make()
        app.agents.update("claude", self.waiting(since=time.time() - 600))
        app.agents.update("claude", self.waiting(since=time.time() - 600))
        self.notes.clear()
        for _ in range(2):
            app.agents.update("claude", self.state(False))
        self.assertIn("10 min", self.notes[-1][1])
        self.assertFalse(app.agents.waiting("claude"))

    def test_the_flag_reaches_what_gets_drawn_when_waiting(self):
        # (In the old file this test shared its name with the one below, so only one of the two ever ran.)
        app = self.make()
        for _ in range(2):
            app.agents.update("claude", self.waiting())
        single = app.agents.flag("claude", usage())
        self.assertEqual((single.working, single.waiting), (True, True))
        other = app.agents.flag("codex", usage(title="Codex"))
        self.assertEqual((other.working, other.waiting), (False, False))
        panels = app.agents.flag("split", [usage(), usage(title="Codex")])
        self.assertEqual([(p.working, p.waiting) for p in panels], [(True, True), (False, False)])

    def test_the_flag_reaches_what_gets_drawn(self):
        app = self.make()
        app.agents.agent = {"claude": {"working": True}}
        original = usage()
        single = app.agents.flag("claude", original)
        self.assertTrue(single.working)
        self.assertFalse(app.agents.flag("codex", usage(title="Codex")).working)
        panels = app.agents.flag("split", [usage(), usage(title="Codex")])
        self.assertEqual([p.working for p in panels], [True, False])
        self.assertFalse(original.working, "the original reading isn't modified")

    def test_the_tooltip_names_each_agent_that_is_going(self):
        app = self.make()
        for key in app.providers:
            app.agents.update(key, self.state(True, since=time.time() - 30))
        tooltip = app.tooltip()
        self.assertIn("Claude trabajando", tooltip)
        self.assertIn("Codex trabajando", tooltip)


if __name__ == "__main__":
    unittest.main()
