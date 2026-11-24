"""Is Claude Code working, waiting for you, or idle? Judged from the tail of its local log."""
import json
import os
import tempfile
import time
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

from geekmagic.adapters.providers import claude_logs, logs

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

    # entries as Claude Code writes them
    @staticmethod
    def prompt(ago, cwd="C:\\Work\\Acme App"):
        return json.dumps({"type": "user", "timestamp": stamp(ago), "cwd": cwd, "message": {"role": "user", "content": "do it"}})

    @staticmethod
    def reply(ago, stop, cwd="C:\\Work\\Acme App"):
        return json.dumps({"type": "assistant", "timestamp": stamp(ago), "cwd": cwd, "message": {"id": "m", "stop_reason": stop, "usage": {}}})

    @staticmethod
    def tool_result(ago, cwd="C:\\Work\\Acme App"):
        return json.dumps({"type": "user", "timestamp": stamp(ago), "cwd": cwd, "message": {"content": [{"type": "tool_result"}]}})

    @staticmethod
    def noise(ago):  # entries that aren't part of the conversation
        return [json.dumps({"type": "attachment", "timestamp": stamp(ago)}), json.dumps({"type": "file-history-snapshot"}),
                json.dumps({"type": "ai-title"}), "not even json"]


class ClaudeTests(Logs):
    def state(self):
        return claude_logs.agent_state(time.time(), self.root)

    def test_a_turn_in_progress_is_working(self):
        self.write("p/s.jsonl", [self.prompt(100), self.reply(95, "tool_use"), self.tool_result(90), self.reply(5, "tool_use")], age=5)
        state = self.state()
        self.assertTrue(state["working"])
        self.assertEqual(state["project"], "Acme App")
        self.assertAlmostEqual(time.time() - state["since"], 100, delta=3)

    def test_a_finished_reply_means_idle(self):
        self.write("p/s.jsonl", [self.prompt(100), self.reply(95, "tool_use"), self.tool_result(90), self.reply(20, "end_turn")], age=20)
        state = self.state()
        self.assertFalse(state["working"])
        self.assertIsNone(state["since"])
        self.assertEqual(state["project"], "Acme App")

    def test_other_ways_a_reply_can_end_a_turn(self):
        for stop in ("stop_sequence", "max_tokens", "refusal"):
            self.write(f"{stop}.jsonl", [self.prompt(30), self.reply(10, stop)], age=10)
        self.assertFalse(self.state()["working"])

    def test_a_prompt_just_sent_is_working_before_any_reply_is_written(self):
        self.write("p/s.jsonl", [self.reply(300, "end_turn"), self.prompt(20)], age=20)
        state = self.state()
        self.assertTrue(state["working"])
        self.assertAlmostEqual(time.time() - state["since"], 20, delta=3, msg="the run starts at the new prompt")

    def test_the_run_starts_after_the_last_finished_reply(self):
        self.write("p/s.jsonl", [self.prompt(900), self.reply(890, "end_turn"), self.prompt(200), self.reply(190, "tool_use"),
                                 self.tool_result(150), self.reply(8, "tool_use")], age=8)
        self.assertAlmostEqual(time.time() - self.state()["since"], 200, delta=3)

    def test_a_silent_turn_is_abandoned_after_a_while_but_a_running_tool_is_given_longer(self):
        self.write("a.jsonl", [self.prompt(500), self.tool_result(400)], age=400)  # nobody answered for 400 s
        self.assertFalse(self.state()["working"])
        os.remove(self.root / "a.jsonl")
        self.write("b.jsonl", [self.prompt(500), self.reply(400, "tool_use")], age=400)  # a long command is running
        self.assertTrue(self.state()["working"])
        os.remove(self.root / "b.jsonl")
        self.write("c.jsonl", [self.prompt(900), self.reply(700, "tool_use")], age=700)  # even that has a limit
        self.assertFalse(self.state()["working"])

    def test_logs_untouched_for_a_while_are_not_even_read(self):
        self.write("old.jsonl", [self.prompt(5000), self.reply(4990, "tool_use")], age=logs.SCAN_AGE + 60)
        self.assertEqual(self.state(), IDLE)

    def test_non_conversation_entries_are_ignored(self):
        self.write("p/s.jsonl", [self.prompt(60), self.reply(50, "tool_use"), *self.noise(40), self.tool_result(10), *self.noise(5)], age=5)
        self.assertTrue(self.state()["working"])
        self.write("q/t.jsonl", [self.prompt(60), self.reply(10, "end_turn"), *self.noise(5)], age=5)
        self.assertTrue(self.state()["working"], "(the other log is still going)")

    def test_a_sub_agent_still_running_keeps_the_session_working(self):
        self.write("p/s.jsonl", [self.prompt(300), self.reply(250, "tool_use")], age=250)
        self.write("p/s/subagents/agent-1.jsonl", [self.prompt(120), self.reply(3, "tool_use")], age=3)
        self.assertTrue(self.state()["working"])

    def test_with_two_runs_the_longest_one_is_reported(self):
        self.write("old.jsonl", [self.prompt(300), self.reply(5, "tool_use", cwd="/p/one")], age=5)
        self.write("new.jsonl", [self.prompt(20), self.reply(4, "tool_use", cwd="/p/two")], age=4)
        self.assertEqual(self.state()["project"], "one")

    def test_nothing_to_read(self):
        self.assertEqual(claude_logs.agent_state(time.time(), self.root / "nope"), IDLE)
        self.assertEqual(self.state(), IDLE)
        self.write("empty.jsonl", [], age=1)
        self.assertEqual(self.state(), IDLE)

    def test_a_huge_log_is_only_read_from_the_end(self):
        filler = [self.reply(5000 - i, "tool_use") for i in range(4000)]
        path = self.write("big.jsonl", [self.prompt(5100), *filler, self.reply(4, "tool_use")], age=4)
        self.assertGreater(path.stat().st_size, logs.TAIL_BYTES)
        self.assertTrue(self.state()["working"])


def tool_call(ago, name, call_id, cwd="C:\\Work\\Acme App"):
    """An assistant entry asking to run one tool (the way Claude Code writes it)."""
    return json.dumps({"type": "assistant", "timestamp": stamp(ago), "cwd": cwd,
                       "message": {"id": "m" + call_id, "stop_reason": "tool_use", "usage": {},
                                   "content": [{"type": "tool_use", "id": call_id, "name": name, "input": {}}]}})


def tool_answer(ago, call_id, cwd="C:\\Work\\Acme App"):
    return json.dumps({"type": "user", "timestamp": stamp(ago), "cwd": cwd,
                       "message": {"content": [{"type": "tool_result", "tool_use_id": call_id}]}})


class ClaudeWaitingTests(Logs):
    def state(self):
        return claude_logs.agent_state(time.time(), self.root)

    def test_asking_you_a_question_is_waiting_at_once(self):
        self.write("s.jsonl", [self.prompt(60), tool_call(3, "AskUserQuestion", "q1")], age=3)
        state = self.state()
        self.assertTrue(state["waiting"] and state["working"])
        self.assertEqual(state["waiting_for"], "question")
        self.assertEqual(state["project"], "Acme App")

    def test_a_plan_waiting_for_approval(self):
        self.write("s.jsonl", [self.prompt(60), tool_call(3, "ExitPlanMode", "p1")], age=3)
        self.assertEqual(self.state()["waiting_for"], "plan")

    def test_a_question_left_unanswered_for_a_long_while_is_still_waiting(self):
        self.write("s.jsonl", [self.prompt(3000), tool_call(2400, "AskUserQuestion", "q1")], age=2400)  # 40 minutes
        self.assertTrue(self.state()["waiting"], "you were away: it must not stop waiting just because time passed")

    def test_answering_it_ends_the_wait(self):
        self.write("s.jsonl", [self.prompt(60), tool_call(30, "AskUserQuestion", "q1"), tool_answer(5, "q1")], age=5)
        state = self.state()
        self.assertFalse(state["waiting"])
        self.assertTrue(state["working"], "back to work")

    def test_an_instant_tool_pending_for_a_while_is_waiting_for_permission(self):
        for tool in ("Edit", "Write", "Read", "Grep"):
            logs._summary_cache.clear()
            self.write("s.jsonl", [self.prompt(120), tool_call(40, tool, "e1")], age=40)
            state = self.state()
            self.assertEqual(state["waiting_for"], f"approval ({tool})", tool)

    def test_an_instant_tool_that_was_only_just_called_is_still_working(self):
        self.write("s.jsonl", [self.prompt(60), tool_call(5, "Edit", "e1")], age=5)
        state = self.state()
        self.assertTrue(state["working"])
        self.assertFalse(state["waiting"])

    def test_a_long_command_is_never_taken_for_a_permission_prompt(self):
        for tool in ("Bash", "PowerShell", "Agent", "mcp__server__tool"):
            logs._summary_cache.clear()
            self.write("s.jsonl", [self.prompt(500), tool_call(300, tool, "b1")], age=300)  # five quiet minutes
            state = self.state()
            self.assertTrue(state["working"], tool)
            self.assertFalse(state["waiting"], f"{tool}: a build looks exactly like this")

    def test_with_several_calls_in_flight_only_the_unanswered_ones_count(self):
        self.write("s.jsonl", [self.prompt(120), tool_call(60, "Read", "r1"), tool_call(59, "Edit", "e1"), tool_answer(58, "r1"),
                               tool_call(57, "Bash", "b1"), tool_answer(56, "b1")], age=56)
        self.assertEqual(self.state()["waiting_for"], "approval (Edit)", "r1 and b1 were answered; e1 was not")

    def test_a_finished_reply_means_nothing_is_waiting_even_with_a_dangling_call(self):
        self.write("s.jsonl", [self.prompt(100), tool_call(80, "AskUserQuestion", "q1"), self.reply(20, "end_turn")], age=20)
        self.assertFalse(self.state()["waiting"])
        self.assertFalse(self.state()["working"])

    def test_a_question_forgotten_for_hours_is_not_waiting_any_more(self):
        self.write("s.jsonl", [self.prompt(40000), tool_call(39000, "AskUserQuestion", "q1")], age=logs.WAIT_MAX_AGE + 120)
        self.assertEqual(self.state(), IDLE)

    def test_a_session_waiting_for_you_is_reported_before_one_that_is_just_working(self):
        self.write("busy.jsonl", [self.prompt(300), self.reply(2, "tool_use", cwd="/p/busy")], age=2)
        self.write("asking.jsonl", [self.prompt(300), tool_call(100, "AskUserQuestion", "q1", cwd="/p/asking")], age=100)
        state = self.state()
        self.assertEqual((state["waiting"], state["project"]), (True, "asking"))

    def test_the_decision_moves_with_the_clock_without_reading_the_log_again(self):
        path = self.write("s.jsonl", [self.prompt(60), tool_call(5, "Edit", "e1")], age=5)
        summary = claude_logs._summary(path)
        now = time.time()
        self.assertFalse(claude_logs._decide(summary, now)["waiting"], "just called")
        self.assertTrue(claude_logs._decide(summary, now + 25)["waiting"], "25 s later, still unanswered")

    def test_logs_are_only_read_again_when_they_change(self):
        path = self.write("s.jsonl", [self.prompt(60), tool_call(5, "Edit", "e1")], age=5)
        with patch.object(claude_logs, "_summary", wraps=claude_logs._summary) as read:
            for _ in range(3):
                claude_logs.agent_state(time.time(), self.root)
            self.assertEqual(read.call_count, 1)
            path.write_text(path.read_text() + "\n" + tool_answer(1, "e1"), encoding="utf-8")
            self.assertFalse(claude_logs.agent_state(time.time(), self.root)["waiting"])
            self.assertEqual(read.call_count, 2)


def sid_entry(kind, ago, sid, cwd, stop=None):
    """A conversation entry of a given session, in a given project."""
    return json.dumps({"type": kind, "timestamp": stamp(ago), "sessionId": sid, "cwd": cwd,
                       "message": {"stop_reason": stop, "usage": {}}})


class SeveralSessionsTests(Logs):
    """Working in several projects at once: each session is followed on its own."""

    def sessions(self):
        return {s["project"]: s for s in claude_logs.agent_state(time.time(), self.root)["sessions"]}

    def test_every_session_is_listed_with_its_own_project_and_state(self):
        self.write("one.jsonl", [sid_entry("user", 60, "S1", "C:\\Work\\Alpha"), sid_entry("assistant", 3, "S1", "C:\\Work\\Alpha", "tool_use")], age=3)
        self.write("two.jsonl", [sid_entry("user", 90, "S2", "C:\\Work\\Beta"), sid_entry("assistant", 40, "S2", "C:\\Work\\Beta", "end_turn")], age=40)
        self.write("three.jsonl", [sid_entry("user", 80, "S3", "C:\\Work\\Gamma"),
                                   tool_call(20, "AskUserQuestion", "q1", cwd="C:\\Work\\Gamma").replace('"cwd"', '"sessionId": "S3", "cwd"')], age=20)
        found = self.sessions()
        self.assertEqual(sorted(found), ["Alpha", "Beta", "Gamma"])
        self.assertEqual((found["Alpha"]["working"], found["Alpha"]["waiting"]), (True, False))
        self.assertEqual((found["Beta"]["working"], found["Beta"]["waiting"]), (False, False))
        self.assertEqual((found["Gamma"]["working"], found["Gamma"]["waiting"]), (True, True))
        self.assertEqual(found["Gamma"]["waiting_for"], "question")
        self.assertEqual(len({s["id"] for s in found.values()}), 3)

    def test_the_overall_state_is_still_the_most_telling_one(self):
        self.write("one.jsonl", [sid_entry("user", 60, "S1", "/p/alpha"), sid_entry("assistant", 3, "S1", "/p/alpha", "tool_use")], age=3)
        self.write("two.jsonl", [sid_entry("user", 80, "S2", "/p/beta"),
                                 tool_call(20, "AskUserQuestion", "q1", cwd="/p/beta").replace('"cwd"', '"sessionId": "S2", "cwd"')], age=20)
        state = claude_logs.agent_state(time.time(), self.root)
        self.assertEqual((state["waiting"], state["project"]), (True, "beta"))
        self.assertNotIn("id", state, "the overall state isn't one session")

    def test_a_sessions_sub_agent_logs_are_merged_into_it(self):
        self.write("p/main.jsonl", [sid_entry("user", 300, "S1", "/p/alpha"), sid_entry("assistant", 250, "S1", "/p/alpha", "tool_use")], age=250)
        self.write("p/S1/subagents/agent-1.jsonl", [sid_entry("user", 100, "S1", "/p/alpha"), sid_entry("assistant", 2, "S1", "/p/alpha", "tool_use")], age=2)
        sessions = claude_logs.agent_state(time.time(), self.root)["sessions"]
        self.assertEqual(len(sessions), 1, "one session, not two")
        self.assertEqual((sessions[0]["project"], sessions[0]["working"]), ("alpha", True))

    def test_two_sessions_in_the_same_project_stay_separate(self):
        self.write("a.jsonl", [sid_entry("user", 60, "S1", "/p/same"), sid_entry("assistant", 3, "S1", "/p/same", "tool_use")], age=3)
        self.write("b.jsonl", [sid_entry("user", 60, "S2", "/p/same"), sid_entry("assistant", 4, "S2", "/p/same", "tool_use")], age=4)
        self.assertEqual(len(claude_logs.agent_state(time.time(), self.root)["sessions"]), 2)

    def test_a_session_whose_log_has_no_session_id_is_named_after_its_file(self):
        self.write("abc123.jsonl", [self.prompt(60), self.reply(3, "tool_use")], age=3)
        self.assertEqual(claude_logs.agent_state(time.time(), self.root)["sessions"][0]["id"], "abc123")



if __name__ == "__main__":
    unittest.main()
