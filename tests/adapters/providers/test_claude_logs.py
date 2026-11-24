"""Claude Code's local logs: how its requests are read and counted, and how its models are named."""
import json
import os
import tempfile
import time
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import Mock, patch

from geekmagic.adapters.providers import claude_logs, logs
from geekmagic.core.activity import TOP


def stamp(when: datetime) -> str:
    return when.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000Z")


def ago(**delta) -> datetime:
    return datetime.now().astimezone() - timedelta(**delta)


class LogFixture(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        logs._event_cache.clear()

    def claude_log(self, name, entries, mtime=None):
        """entries: (datetime, session id, message id) assistant replies, one JSON line each."""
        lines = [json.dumps({"type": "assistant", "timestamp": stamp(when), "sessionId": sid,
                             "message": {"id": mid, "usage": {"input_tokens": 1, "output_tokens": 1}}})
                 for when, sid, mid in entries]
        return self.write(name, lines, mtime)

    def write(self, name, lines, mtime=None):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(lines), encoding="utf-8")
        if mtime is not None:
            os.utime(path, (mtime, mtime))
        return path


class StatsTests(LogFixture):
    def test_counts_replies_and_sessions_per_window(self):
        self.claude_log("proj/a.jsonl", [(ago(hours=1), "A", "m1"), (ago(hours=3), "A", "m2"), (ago(hours=30), "A", "m3")])
        self.claude_log("proj/b.jsonl", [(ago(days=3), "B", "m4"), (ago(days=3, hours=1), "B", "m5")])
        stats = claude_logs.stats(time.time(), self.root)
        self.assertEqual(stats["24h"], {"requests": 2, "sessions": 1})
        self.assertEqual(stats["7d"], {"requests": 5, "sessions": 2})

    def test_a_reply_written_in_several_blocks_counts_once(self):
        when = ago(hours=1)
        self.claude_log("a.jsonl", [(when, "A", "same"), (when, "A", "same"), (when, "A", "other")])
        self.assertEqual(claude_logs.stats(time.time(), self.root)["24h"]["requests"], 2)

    def test_subagent_logs_belong_to_the_parent_session(self):
        self.claude_log("proj/parent.jsonl", [(ago(hours=2), "P", "m1")])
        self.claude_log("proj/P/subagents/agent-1.jsonl", [(ago(hours=2), "P", "m2"), (ago(hours=1), "P", "m3")])
        self.assertEqual(claude_logs.stats(time.time(), self.root)["24h"], {"requests": 3, "sessions": 1})

    def test_only_assistant_replies_with_usage_count(self):
        self.write("a.jsonl", [
            json.dumps({"type": "user", "timestamp": stamp(ago(hours=1)), "message": {"usage": {}}}),
            json.dumps({"type": "assistant", "timestamp": stamp(ago(hours=1)), "message": {"id": "x"}}),  # no usage
            json.dumps({"type": "assistant", "timestamp": "not a time", "message": {"id": "y", "usage": {}}}),
            "this line is not json {{{ usage assistant",
            json.dumps({"type": "assistant", "timestamp": stamp(ago(hours=1)), "sessionId": "S", "message": {"id": "ok", "usage": {}}}),
        ])
        self.assertEqual(claude_logs.stats(time.time(), self.root)["24h"], {"requests": 1, "sessions": 1})

    def test_a_session_without_an_id_is_named_after_its_file(self):
        self.write("abc123.jsonl", [json.dumps({"type": "assistant", "timestamp": stamp(ago(hours=1)), "message": {"id": "m", "usage": {}}})])
        self.assertEqual([e[1] for e in claude_logs.read_events(self.root / "abc123.jsonl")], ["abc123"])

    def test_a_missing_folder_means_no_stats(self):
        self.assertIsNone(claude_logs.stats(time.time(), self.root / "nope"))

    def test_no_activity_at_all_is_zeroes_not_nothing(self):
        stats = claude_logs.stats(time.time(), self.root)
        self.assertEqual(stats["24h"], {"requests": 0, "sessions": 0})
        self.assertEqual([d["requests"] for d in stats["days"]], [0] * 7)


class BreakdownTests(LogFixture):
    def claude_entry(self, when, sid, mid, cwd, model):
        return json.dumps({"type": "assistant", "timestamp": stamp(when), "sessionId": sid, "cwd": cwd,
                           "message": {"id": mid, "model": model, "usage": {"input_tokens": 1}}})

    def test_requests_are_split_by_project_and_model(self):
        self.write("a.jsonl", [
            self.claude_entry(ago(hours=1), "A", "1", "C:\\Work\\Alpha", "claude-opus-5-5"),
            self.claude_entry(ago(hours=2), "A", "2", "C:\\Work\\Alpha", "claude-opus-5-5"),
            self.claude_entry(ago(hours=3), "A", "3", "C:\\Work\\Alpha", "claude-sonnet-5"),
            self.claude_entry(ago(hours=4), "B", "4", "C:\\Work\\Beta", "claude-sonnet-5"),
            self.claude_entry(ago(days=3), "B", "5", "C:\\Work\\Beta", "claude-haiku-4-5-20251001"),
            self.claude_entry(ago(days=10), "C", "6", "C:\\Work\\Old", "claude-opus-5-5"),  # outside the week
        ])
        stats = claude_logs.stats(time.time(), self.root)
        self.assertEqual(stats["total"], 5)
        self.assertEqual([(p["name"], p["requests"]) for p in stats["projects"]], [("Alpha", 3), ("Beta", 2)])
        self.assertEqual({m["name"]: m["requests"] for m in stats["models"]}, {"Opus 5.5": 2, "Sonnet 5": 2, "Haiku 4.5": 1})

    def test_the_busiest_comes_first_and_only_the_top_are_kept(self):
        self.write("a.jsonl", [self.claude_entry(ago(minutes=i), "A", str(i), f"/p/project{i % 9}", "claude-opus-5-5") for i in range(60)])
        projects = claude_logs.stats(time.time(), self.root)["projects"]
        self.assertEqual(len(projects), TOP)
        self.assertEqual([p["requests"] for p in projects], sorted((p["requests"] for p in projects), reverse=True))

    def test_synthetic_messages_are_not_requests(self):
        self.write("a.jsonl", [self.claude_entry(ago(hours=1), "A", "1", "/p/x", "<synthetic>"),
                               self.claude_entry(ago(hours=1), "A", "2", "/p/x", "claude-opus-5-5")])
        self.assertEqual(claude_logs.stats(time.time(), self.root)["24h"]["requests"], 1)


class ModelLabelTests(unittest.TestCase):
    def test_claude_model_names_become_family_and_version(self):
        names = {"claude-opus-5-5": "Opus 5.5", "claude-sonnet-5": "Sonnet 5", "claude-opus-4-1-20250805": "Opus 4.1",
                 "claude-haiku-4-5-20251001": "Haiku 4.5", "claude-3-5-sonnet-20241022": "Sonnet 3.5",
                 "claude-3-opus-20240229": "Opus 3", "claude-sonnet-4-20250514": "Sonnet 4"}
        for raw, label in names.items():
            self.assertEqual(claude_logs.model_label(raw), label, raw)

    def test_other_models_are_shown_as_they_are(self):
        self.assertEqual(claude_logs.model_label("gpt-6.1-sol"), "gpt-6.1-sol")
        self.assertEqual(claude_logs.model_label(None), "(unknown)")


class CacheTests(LogFixture):
    def test_unchanged_logs_are_not_read_twice_but_grown_ones_are(self):
        self.claude_log("a.jsonl", [(ago(hours=1), "A", "m1")])
        read = Mock(wraps=claude_logs.read_events)
        logs.collect(self.root, read, time.time())
        logs.collect(self.root, read, time.time())
        self.assertEqual(read.call_count, 1, "second pass served from the cache")
        self.claude_log("a.jsonl", [(ago(hours=1), "A", "m1"), (ago(minutes=5), "A", "m2")])
        self.assertEqual(len(logs.collect(self.root, read, time.time())), 2)
        self.assertEqual(read.call_count, 2)

    def test_logs_untouched_since_before_the_chart_are_skipped_without_reading(self):
        self.claude_log("old.jsonl", [(ago(hours=1), "A", "m1")], mtime=time.time() - 30 * 86400)
        read = Mock()
        self.assertEqual(logs.collect(self.root, read, time.time()), [])
        read.assert_not_called()

    def test_an_unreadable_log_is_skipped(self):
        self.claude_log("a.jsonl", [(ago(hours=1), "A", "m1")])
        with patch.object(claude_logs, "read_events", side_effect=OSError("locked")):
            self.assertEqual(logs.collect(self.root, claude_logs.read_events, time.time()), [])


if __name__ == "__main__":
    unittest.main()
