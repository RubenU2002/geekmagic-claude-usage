"""Claude Code's local logs (`~/.claude/projects/**/*.jsonl`): what you did with it, and whether it is working right now.

Claude Code appends an `assistant` entry for every reply (its `stop_reason` is "tool_use" while it wants to run a tool
and "end_turn" when it is done) and a `user` entry for the prompt and for each tool result. So a log whose last
conversation entry is anything but a finished reply is a turn in progress. It is *waiting for you* when a tool call has
no result and that tool is one that only waits on a person: `AskUserQuestion` (it asked you something) and
`ExitPlanMode` (it wants your approval of a plan) at once, and a tool that normally returns in milliseconds (`Read`,
`Edit`, `Write`...) that has been pending for a while, which means it is waiting for you to allow it. A long `Bash`
command looks the same from the log as a permission prompt, so it is never taken for waiting.
"""

from __future__ import annotations

import json
import os
import re
from pathlib import Path

from geekmagic.adapters.providers.logs import TOOL_MAX_AGE, TURN_MAX_AGE, WAIT_MAX_AGE, collect, epoch, scan_state, tail_lines
from geekmagic.core.activity import Event, project_label, summarize

PERMISSION_WAIT = 20  # an instant tool still pending after this many seconds is waiting for you to allow it
CLAUDE_DIR = Path(os.environ.get("CLAUDE_CONFIG_DIR") or Path.home() / ".claude") / "projects"


_CLAUDE_MODEL_RE = re.compile(r"^claude-(opus|sonnet|haiku)-(\d+)(?:-(\d{1,2}))?(?!\d)")


_CLAUDE_OLD_MODEL_RE = re.compile(r"^claude-(\d+)(?:-(\d+))?-(opus|sonnet|haiku)")


def model_label(model: str | None) -> str:
    """"claude-opus-5-5" -> "Opus 5.5", "claude-sonnet-5" -> "Sonnet 5", older "claude-3-5-sonnet-2024..." ->
    "Sonnet 3.5"; any other name (Codex's) is shown as is."""
    if not model:
        return "(unknown)"
    new = _CLAUDE_MODEL_RE.match(model)
    if new:
        family, major, minor = new.groups()
        return f"{family.title()} {major}" + (f".{minor}" if minor else "")
    old = _CLAUDE_OLD_MODEL_RE.match(model)
    if old:
        major, minor, family = old.groups()
        return f"{family.title()} {major}" + (f".{minor}" if minor else "")
    return model


def read_events(path: Path) -> list[Event]:
    """Every model call in one Claude Code log: an `assistant` entry with token usage. A reply written in several
    blocks repeats its message id, so ids are what get counted. The session is the entry's own sessionId (sub-agents
    log under their parent's)."""
    found = []
    with path.open(encoding="utf-8", errors="replace") as f:
        for line in f:
            if '"usage"' not in line or '"assistant"' not in line:
                continue
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            message = entry.get("message")
            if entry.get("type") != "assistant" or not isinstance(message, dict) or "usage" not in message:
                continue
            if message.get("model") == "<synthetic>":  # Claude Code's own notices, not a model call
                continue
            when = epoch(entry.get("timestamp") or "")
            if when is not None:
                found.append((when, entry.get("sessionId") or path.stem, message.get("id") or entry.get("uuid"),
                              project_label(entry.get("cwd")), model_label(message.get("model"))))
    return found


def stats(now_epoch: float, root: Path | None = None) -> dict | None:
    root = root or CLAUDE_DIR
    return summarize(collect(root, read_events, now_epoch), now_epoch) if root.is_dir() else None


FINISHED = {"end_turn", "stop_sequence", "max_tokens", "refusal", "pause_turn"}  # a reply that ends the turn


ASKS = {"AskUserQuestion": "question", "ExitPlanMode": "plan"}  # tools that exist to wait for you


INSTANT_TOOLS = {"Read", "Edit", "Write", "MultiEdit", "NotebookEdit", "Glob", "Grep", "LS", "TodoWrite"}


def _summary(path: Path) -> dict | None:
    turn = []  # (time, kind, stop_reason) of the user/assistant entries, in order
    pending: dict[str, tuple[str, float]] = {}  # tool call id -> (tool name, when it was called), until its result
    project = session = None
    for line in tail_lines(path):
        if '"user"' not in line and '"assistant"' not in line:
            continue
        try:
            entry = json.loads(line)
        except ValueError:
            continue
        kind = entry.get("type")
        when = epoch(entry.get("timestamp"))
        if kind not in ("user", "assistant") or when is None:
            continue
        message = entry.get("message") if isinstance(entry.get("message"), dict) else {}
        turn.append((when, kind, message.get("stop_reason")))
        project = project_label(entry["cwd"]) if entry.get("cwd") else project
        session = entry.get("sessionId") or session  # a sub-agent's log carries its parent session's id
        content = message.get("content")
        for block in content if isinstance(content, list) else []:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "tool_use" and block.get("id"):
                pending[block["id"]] = (block.get("name") or "", when)
            elif block.get("type") == "tool_result":
                pending.pop(block.get("tool_use_id"), None)
    if not turn:
        return None
    begin = 0
    for i, (_, kind, stop) in enumerate(turn):  # the turn began right after the last reply that finished one
        if kind == "assistant" and stop in FINISHED:
            begin = i + 1
    return {"turn": turn, "begin": turn[min(begin, len(turn) - 1)][0], "pending": pending, "project": project,
            "session": session or path.stem}


def _decide(summary: dict | None, now_epoch: float) -> dict | None:
    if not summary:
        return None
    last_time, last_kind, last_stop = summary["turn"][-1]
    age = now_epoch - last_time
    finished = last_kind == "assistant" and last_stop in FINISHED
    waiting_for = None
    if not finished and age <= WAIT_MAX_AGE:
        for name, when in summary["pending"].values():
            if name in ASKS:
                waiting_for = ASKS[name]
                break
            if name in INSTANT_TOOLS and now_epoch - when >= PERMISSION_WAIT:
                waiting_for = f"approval ({name})"
    running_tool = last_kind == "assistant" and last_stop in ("tool_use", None)  # None: still being written
    working = waiting_for is not None or (not finished and age <= (TOOL_MAX_AGE if running_tool else TURN_MAX_AGE))
    return {"id": summary["session"], "working": working, "waiting": waiting_for is not None, "waiting_for": waiting_for,
            "since": summary["begin"] if working else None, "last": last_time, "project": summary["project"]}


def agent_state(now_epoch: float, root: Path | None = None) -> dict:
    return scan_state(root or CLAUDE_DIR, _summary, _decide, now_epoch)
