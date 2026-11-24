"""Several sessions of one agent at once: merging the logs of a session, and describing how long it has been going."""
import unittest

from geekmagic.core.sessions import duration_text, merge_sessions


def state(session, last, project="Alpha", working=False, waiting=False, waiting_for=None, since=None):
    """What one log says about its session, as the readers' decisions produce it."""
    return {"id": session, "working": working, "waiting": waiting, "waiting_for": waiting_for, "since": since,
            "last": last, "project": project}


class MergeTests(unittest.TestCase):
    def test_every_session_is_listed_once_with_its_own_project(self):
        merged = merge_sessions([state("S1", 10, "Alpha"), state("S2", 20, "Beta"), state("S3", 30, "Gamma")])
        self.assertEqual({s["id"]: s["project"] for s in merged}, {"S1": "Alpha", "S2": "Beta", "S3": "Gamma"})

    def test_a_sub_agent_log_is_merged_into_its_session(self):
        merged = merge_sessions([state("S1", 10, working=True, since=5), state("S1", 90, working=True, since=60)])
        self.assertEqual(len(merged), 1, "one session, not two")
        self.assertEqual((merged[0]["working"], merged[0]["since"], merged[0]["last"]), (True, 5, 90))

    def test_a_session_works_if_any_of_its_logs_does(self):
        merged = merge_sessions([state("S1", 10, working=True, since=5), state("S1", 90)])
        self.assertTrue(merged[0]["working"])
        self.assertEqual(merged[0]["since"], 5)

    def test_a_session_waits_if_any_of_its_logs_does(self):
        merged = merge_sessions([state("S1", 10), state("S1", 90, working=True, waiting=True, waiting_for="question", since=40)])
        self.assertEqual((merged[0]["waiting"], merged[0]["waiting_for"]), (True, "question"))

    def test_an_idle_session_has_no_start_or_reason(self):
        merged = merge_sessions([state("S1", 10, since=3, waiting_for="left over")])
        self.assertEqual((merged[0]["working"], merged[0]["waiting"], merged[0]["waiting_for"], merged[0]["since"]),
                         (False, False, None, None))

    def test_the_freshest_log_names_the_project(self):
        merged = merge_sessions([state("S1", 90, "Fresh"), state("S1", 10, "Old")])
        self.assertEqual(merged[0]["project"], "Fresh")

    def test_a_log_that_does_not_know_its_project_does_not_erase_it(self):
        merged = merge_sessions([state("S1", 10, "Alpha"), state("S1", 90, None)])
        self.assertEqual(merged[0]["project"], "Alpha")

    def test_two_sessions_in_the_same_project_stay_separate(self):
        merged = merge_sessions([state("S1", 10, "same"), state("S2", 20, "same")])
        self.assertEqual(len(merged), 2)

    def test_no_logs_no_sessions(self):
        self.assertEqual(merge_sessions([]), [])


class DurationTests(unittest.TestCase):
    def test_durations(self):
        self.assertEqual([duration_text(x) for x in (0, 5, 59.6, 60, 240, 3599, 3600, 3900, -4)],
                         ["0 s", "5 s", "1 min", "1 min", "4 min", "59 min", "1 h 00 min", "1 h 05 min", "0 s"])


if __name__ == "__main__":
    unittest.main()
