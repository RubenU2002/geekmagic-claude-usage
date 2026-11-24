"""Is Codex working, waiting for you, or idle? Judged from the tail of its local log."""
import json
import os
import tempfile
import time
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

from geekmagic.adapters.providers import codex_logs, logs

IDLE = {"working": False, "waiting": False, "waiting_for": None, "since": None, "last": None, "project": None,
        "sessions": []}


def stamp(seconds_ago: float) -> str:
    return (datetime.now(timezone.utc) - timedelta(seconds=seconds_ago)).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"


class Logs(unittest.TestCase):
    def setUp(self):
        logs._summary_cache.clear()
        self.root = Path(tempfile.mkdtemp())

    def write(self, name, lines, age=0.0):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(lines), encoding="utf-8")
        mtime = time.time() - age
        os.utime(path, (mtime, mtime))
        return path


class CodexTests(Logs):
    @staticmethod
    def event(ago, kind=None, **payload):
        body = {"timestamp": stamp(ago), "type": "event_msg", "payload": {"type": kind or "message", **payload}}
        return json.dumps(body)

    @staticmethod
    def context(ago, cwd):
        return json.dumps({"timestamp": stamp(ago), "type": "turn_context", "payload": {"cwd": cwd, "model": "gpt-6.1-sol"}})

    def state(self):
        return codex_logs.agent_state(time.time(), self.root)

    def test_a_started_task_is_working(self):
        self.write("r.jsonl", [self.context(100, "C:\\Work\\Alpha"), self.event(95, "task_started"), self.event(30), self.event(4)], age=4)
        state = self.state()
        self.assertTrue(state["working"])
        self.assertEqual(state["project"], "Alpha")
        self.assertAlmostEqual(time.time() - state["since"], 95, delta=3)

    def test_a_completed_or_aborted_task_is_idle(self):
        self.write("a.jsonl", [self.event(100, "task_started"), self.event(20, "task_complete")], age=20)
        self.assertFalse(self.state()["working"])
        os.remove(self.root / "a.jsonl")
        self.write("b.jsonl", [self.event(100, "task_started"), self.event(20, "turn_aborted")], age=20)
        self.assertFalse(self.state()["working"])

    def test_the_latest_marker_wins(self):
        self.write("r.jsonl", [self.event(300, "task_started"), self.event(200, "task_complete"), self.event(50, "task_started"), self.event(3)], age=3)
        self.assertTrue(self.state()["working"])

    def test_an_abandoned_task_stops_counting_after_a_long_silence(self):
        self.write("r.jsonl", [self.event(900, "task_started"), self.event(700)], age=700)
        self.assertFalse(self.state()["working"])

    def test_a_long_run_whose_markers_scrolled_out_of_the_tail_is_judged_by_recent_writes(self):
        busy = [self.event(2000 - i) for i in range(4000)]
        self.write("r.jsonl", [self.event(3000, "task_started"), *busy, self.event(2)], age=2)
        self.assertTrue(self.state()["working"])

    def test_nothing_to_read(self):
        self.assertEqual(codex_logs.agent_state(time.time(), self.root / "nope"), IDLE)
        self.assertEqual(self.state(), IDLE)

    # --- waiting for you (best effort: not every setup records these) ---------------------------------------

    def test_an_approval_request_that_is_the_last_thing_means_waiting(self):
        self.write("r.jsonl", [self.event(100, "task_started"), self.event(30, "exec_command_begin"),
                               self.event(10, "exec_approval_request")], age=10)
        state = self.state()
        self.assertTrue(state["working"] and state["waiting"])
        self.assertEqual(state["waiting_for"], "approval")

    def test_a_question_request_is_waiting_too(self):
        self.write("r.jsonl", [self.event(100, "task_started"), self.event(10, "request_user_input")], age=10)
        self.assertTrue(self.state()["waiting"])

    def test_once_something_else_happens_it_is_no_longer_waiting(self):
        self.write("r.jsonl", [self.event(100, "task_started"), self.event(10, "exec_approval_request"), self.event(3, "exec_command_end")], age=3)
        state = self.state()
        self.assertTrue(state["working"])
        self.assertFalse(state["waiting"])

    def test_a_request_after_the_task_finished_is_not_waiting(self):
        self.write("r.jsonl", [self.event(100, "task_started"), self.event(50, "task_complete"), self.event(10, "exec_approval_request")], age=10)
        self.assertFalse(self.state()["waiting"])

    def test_an_old_unanswered_request_is_taken_for_a_forgotten_session(self):
        self.write("r.jsonl", [self.event(50000, "task_started"), self.event(40000, "exec_approval_request")], age=logs.WAIT_MAX_AGE + 60)
        self.assertEqual(self.state(), IDLE)

    def test_the_event_type_is_read_from_the_payload(self):
        for line, kind in ((self.event(1, "exec_approval_request"), "exec_approval_request"),
                           ('{"timestamp": "x", "payload" : { "type" : "message", "a": 1}}', "message"),
                           ('{"payload": {"other": 1}}', None), ("not json", None)):
            match = codex_logs._PAYLOAD_TYPE_RE.search(line)
            self.assertEqual(match.group(1) if match else None, kind, line)

    def test_sessions_are_separate_per_rollout(self):
        ctx = lambda ago, cwd: json.dumps({"timestamp": stamp(ago), "type": "turn_context", "payload": {"cwd": cwd, "model": "m"}})
        self.write("rollout-1.jsonl", [ctx(60, "/p/alpha"), self.event(50, "task_started"), self.event(3, "token_count")], age=3)
        self.write("rollout-2.jsonl", [ctx(60, "/p/beta"), self.event(50, "task_started"), self.event(30, "task_complete")], age=30)
        sessions = {s["project"]: s for s in codex_logs.agent_state(time.time(), self.root)["sessions"]}
        self.assertEqual((sessions["alpha"]["working"], sessions["beta"]["working"]), (True, False))
        self.assertEqual(sessions["alpha"]["id"], "rollout-1")

    def test_the_summary_moves_with_the_clock_without_reading_the_log_again(self):
        path = self.write("r.jsonl", [self.event(100, "task_started"), self.event(10, "exec_approval_request")], age=10)
        summary = codex_logs._summary(path)
        now = time.time()
        self.assertTrue(codex_logs._decide(summary, now)["waiting"])
        self.assertFalse(codex_logs._decide(summary, now + logs.WAIT_MAX_AGE + 60)["waiting"], "forgotten by then")


if __name__ == "__main__":
    unittest.main()
