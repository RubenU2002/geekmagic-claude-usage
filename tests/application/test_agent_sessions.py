"""Several projects at once: each session is followed on its own, both providers are polled, and an agent change
redraws without querying the usage again."""
import threading
import time
import unittest
from unittest.mock import patch

from geekmagic.application import config
from tests.support import AppTestCase, capture_screen, fake_fetchers, usage


class AgentSessionTests(AppTestCase):
    def state(self, working, since=None, project="Acme App"):
        return {"working": working, "since": since, "last": time.time(), "project": project}

    def session(self, sid, project, working=True, waiting=False, reason=None, since=None):
        return {"id": sid, "project": project, "working": working, "waiting": waiting, "waiting_for": reason,
                "since": since or time.time() - 300, "last": time.time()}

    def overall(self, *sessions):
        """What a provider's agent_state reports: the overall state plus every session."""
        lead = next((s for s in sessions if s["waiting"]), next((s for s in sessions if s["working"]), sessions[0]))
        return {**{k: v for k, v in lead.items() if k != "id"}, "sessions": list(sessions)}

    def test_each_project_is_followed_and_notified_on_its_own(self):
        app = self.make()
        alpha, beta = self.session("A", "Alpha"), self.session("B", "Beta")
        app.agents.update("claude", self.overall(alpha, beta))
        self.assertEqual(app.agents.agent["claude"]["sessions"], 2)
        beta_asks = {**beta, "waiting": True, "reason": "question", "waiting_for": "question"}
        for _ in range(2):
            app.agents.update("claude", self.overall(alpha, beta_asks))
        self.assertEqual(self.notes, [("Claude · Beta", "Te hizo una pregunta")], "it says which project is asking")
        self.assertTrue(app.agents.waiting("claude"))

    def test_two_projects_asking_at_once_each_get_their_own_notification(self):
        app = self.make()
        asking = lambda sid, project: {**self.session(sid, project), "waiting": True, "waiting_for": "question"}
        for _ in range(3):
            app.agents.update("claude", self.overall(asking("A", "Alpha"), asking("B", "Beta")))
        self.assertEqual(sorted(self.notes), [("Claude · Alpha", "Te hizo una pregunta"), ("Claude · Beta", "Te hizo una pregunta")])

    def test_one_project_finishing_is_reported_while_another_keeps_working(self):
        app = self.make()
        alpha, beta = self.session("A", "Alpha", since=time.time() - 400), self.session("B", "Beta", since=time.time() - 50)
        app.agents.update("claude", self.overall(alpha, beta))
        alpha_done = {**alpha, "working": False, "waiting": False, "since": None}
        for _ in range(2):
            app.agents.update("claude", self.overall(alpha_done, beta))
        self.assertEqual(self.notes, [("Claude · Alpha", "Terminó (trabajó 6 min)")])
        self.assertTrue(app.agents.working("claude"), "Beta is still going: the screen keeps saying so")
        self.assertEqual(app.agents.agent["claude"]["sessions"], 1)
        self.assertEqual(app.agents.agent["claude"]["project"], "Beta")

    def test_the_last_one_finishing_clears_the_indicator(self):
        app = self.make()
        alpha = self.session("A", "Alpha", since=time.time() - 400)
        app.agents.update("claude", self.overall(alpha))
        done = {**alpha, "working": False, "since": None}
        for _ in range(2):
            app.agents.update("claude", self.overall(done))
        self.assertFalse(app.agents.working("claude"))
        self.assertEqual(len(self.notes), 1)

    def test_a_session_that_disappears_counts_as_finished(self):
        app = self.make()
        app.agents.update("claude", self.overall(self.session("A", "Alpha", since=time.time() - 200)))
        idle = {"working": False, "waiting": False, "waiting_for": None, "since": None, "last": None, "project": None, "sessions": []}
        for _ in range(2):
            app.agents.update("claude", idle)
        self.assertFalse(app.agents.working("claude"))
        self.assertEqual(self.notes, [("Claude · Alpha", "Terminó (trabajó 3 min)")])
        self.assertEqual(app.agents.tracks["claude"], {}, "forgotten once idle and gone")

    def test_the_needier_project_leads_the_summary_the_screens_use(self):
        app = self.make()
        working = self.session("A", "Alpha")
        asking = {**self.session("B", "Beta"), "waiting": True, "waiting_for": "plan"}
        for _ in range(2):
            app.agents.update("claude", self.overall(working, asking))
        summary = app.agents.agent["claude"]
        self.assertEqual((summary["waiting"], summary["waiting_for"], summary["project"]), (True, "plan", "Beta"))

    def test_sessions_of_the_two_providers_do_not_mix(self):
        app = self.make()
        app.agents.update("claude", self.overall(self.session("S", "Same")))
        app.agents.update("codex", self.overall(self.session("S", "Same")))
        self.assertEqual((app.agents.agent["claude"]["sessions"], app.agents.agent["codex"]["sessions"]), (1, 1))
        done = {**self.session("S", "Same"), "working": False}
        for _ in range(2):
            app.agents.update("codex", self.overall(done))
        self.assertTrue(app.agents.working("claude"))
        self.assertFalse(app.agents.working("codex"))

    def test_polling_reads_both_agents_and_survives_a_failure(self):
        app = self.make()
        app.providers["claude"].agent = lambda now: self.state(True, since=now - 100)
        app.providers["codex"].agent = lambda now: 1 / 0
        app.agents.poll()
        self.assertTrue(app.agents.working("claude"))
        self.assertFalse(app.agents.working("codex"))

    def test_uploads_carry_the_flag(self):
        app = self.make()
        app.agents.agent = {"codex": {"working": True}}
        with capture_screen() as cap:
            app.screen.upload("codex", usage(title="Codex"))
            app.screen.upload("claude", usage())
        self.assertEqual([u.working for u, _ in cap.single], [True, False])

    def test_an_agent_change_redraws_from_what_is_already_read_without_querying_again(self):
        app = self.make()
        queries = []
        with fake_fetchers(claude=lambda: queries.append("claude") or usage(),
                           codex=lambda: queries.append("codex") or usage(title="Codex")), capture_screen():
            app.usage.fetch("claude")
            app.usage.fetch("codex")
            queries.clear()
            self.assertEqual(app.scheduler.update_panels("split", refetch=False), "ok")
            self.assertEqual(queries, [], "redrawn from the last readings")
            app.scheduler.update_panels("split")
            self.assertEqual(sorted(queries), ["claude", "codex"])

    def test_the_agent_watcher_keeps_working_while_paused(self):
        app = self.make()
        app.power.paused = True
        app.agents.update("claude", self.state(True, since=time.time() - 300))
        for _ in range(2):
            app.agents.update("claude", self.state(False))
        self.assertEqual(len(self.notes), 1, "paused or locked is exactly when you want to hear it")

    def test_the_watch_thread_itself_keeps_polling_while_paused(self):
        # the thread (not just `update`): pausing stops the worker, but the looking at the agents' logs goes on
        app = self.make()
        app.power.paused = True
        app.providers["claude"].agent = lambda now: self.state(True, since=now - 100)
        app.providers["codex"].agent = lambda now: self.state(False)
        with patch.object(config, "AGENT_POLL", 0.01):
            thread = threading.Thread(target=app.agents.watch, daemon=True)
            thread.start()
            try:
                deadline = time.time() + 5
                while not app.agents.working("claude") and time.time() < deadline:
                    time.sleep(0.01)
            finally:
                app.stop.set()
                thread.join(5)
        self.assertTrue(app.agents.working("claude"))


if __name__ == "__main__":
    unittest.main()
