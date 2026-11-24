"""Codex's local logs: how its model calls are read and counted."""
import json
import os
import tempfile
import time
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

from geekmagic.adapters.providers import claude_logs, codex_logs, logs


def stamp(when: datetime) -> str:
    return when.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000Z")


def ago(**delta) -> datetime:
    return datetime.now().astimezone() - timedelta(**delta)


class LogFixture(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        logs._event_cache.clear()

    def claude_log(self, name, entries):
        lines = [json.dumps({"type": "assistant", "timestamp": stamp(when), "sessionId": sid,
                             "message": {"id": mid, "usage": {"input_tokens": 1, "output_tokens": 1}}})
                 for when, sid, mid in entries]
        self.write(name, lines)

    def codex_log(self, name, whens, mtime=None):
        lines = [json.dumps({"timestamp": stamp(when), "type": "event_msg",
                             "payload": {"type": "token_count", "info": {"last_token_usage": {"total_tokens": 5}}}})
                 for when in whens]
        self.write(name, lines, mtime)

    def write(self, name, lines, mtime=None):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(lines), encoding="utf-8")
        if mtime is not None:
            os.utime(path, (mtime, mtime))
        return path


class StatsTests(LogFixture):
    def test_counts_model_calls_and_sessions_per_window(self):
        self.codex_log("2026/10/02/today.jsonl", [ago(hours=1), ago(hours=2), ago(hours=30)])
        self.codex_log("2026/09/30/older.jsonl", [ago(days=3), ago(days=3, hours=1)])
        stats = codex_logs.stats(time.time(), self.root)
        self.assertEqual(stats["24h"], {"requests": 2, "sessions": 1})
        self.assertEqual(stats["7d"], {"requests": 5, "sessions": 2})

    def test_events_without_token_info_are_not_requests(self):
        self.write("a.jsonl", [
            json.dumps({"timestamp": stamp(ago(hours=1)), "payload": {"type": "token_count", "info": None}}),
            json.dumps({"timestamp": stamp(ago(hours=1)), "payload": {"type": "token_count", "info": {"x": 1}}}),
            json.dumps({"timestamp": stamp(ago(hours=1)), "payload": {"type": "message"}}),
        ])
        self.assertEqual(codex_logs.stats(time.time(), self.root)["24h"], {"requests": 1, "sessions": 1})

    def test_the_session_is_the_log_file(self):
        self.codex_log("rollout-1.jsonl", [ago(hours=1)])
        self.assertEqual([e[1] for e in codex_logs.read_events(self.root / "rollout-1.jsonl")], ["rollout-1"])

    def test_a_missing_folder_means_no_stats(self):
        self.assertIsNone(codex_logs.stats(time.time(), self.root / "nope"))

    def test_no_activity_at_all_is_zeroes_not_nothing(self):
        stats = codex_logs.stats(time.time(), self.root)
        self.assertEqual(stats["24h"], {"requests": 0, "sessions": 0})
        self.assertEqual([d["requests"] for d in stats["days"]], [0] * 7)


class SameShapeTests(LogFixture):
    def test_both_providers_give_exactly_the_same_kind_of_numbers(self):
        self.claude_log("c/a.jsonl", [(ago(hours=1), "A", "m1")])
        self.codex_log("x/b.jsonl", [ago(hours=1)])
        claude = claude_logs.stats(time.time(), self.root / "c")
        codex = codex_logs.stats(time.time(), self.root / "x")
        self.assertEqual(set(claude), set(codex))
        self.assertEqual(set(claude["24h"]), set(codex["24h"]))
        self.assertEqual([d["date"] for d in claude["days"]], [d["date"] for d in codex["days"]])
        self.assertEqual(claude["24h"], codex["24h"])


class BreakdownTests(LogFixture):
    def test_requests_follow_the_latest_turn_context(self):
        def at(hours):
            return stamp(ago(hours=hours))
        count = {"payload": {"type": "token_count", "info": {"x": 1}}}
        self.write("r.jsonl", [
            json.dumps({"type": "session_meta", "timestamp": at(6), "payload": {"cwd": "C:\\Work\\Alpha"}}),
            json.dumps({"type": "turn_context", "timestamp": at(5), "payload": {"cwd": "C:\\Work\\Alpha", "model": "gpt-5.6-luna"}}),
            json.dumps({"timestamp": at(5), **count}), json.dumps({"timestamp": at(4), **count}),
            json.dumps({"type": "turn_context", "timestamp": at(3), "payload": {"cwd": "c:\\Work\\Beta", "model": "gpt-6.1-sol"}}),
            json.dumps({"timestamp": at(2), **count}),
        ])
        stats = codex_logs.stats(time.time(), self.root)
        self.assertEqual({p["name"]: p["requests"] for p in stats["projects"]}, {"Alpha": 2, "Beta": 1})
        self.assertEqual({m["name"]: m["requests"] for m in stats["models"]}, {"gpt-5.6-luna": 2, "gpt-6.1-sol": 1})

    def test_calls_before_any_context_are_of_an_unknown_project_and_model(self):
        self.codex_log("a.jsonl", [ago(hours=1)])
        stats = codex_logs.stats(time.time(), self.root)
        self.assertEqual([p["name"] for p in stats["projects"]], ["(unknown)"])
        self.assertEqual([m["name"] for m in stats["models"]], ["(unknown)"])


if __name__ == "__main__":
    unittest.main()
