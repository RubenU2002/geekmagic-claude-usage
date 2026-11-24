"""Codex's local logs (`~/.codex/sessions/**/*.jsonl`): what you did with it, and whether it is working right now.

Codex marks every turn with `task_started` and `task_complete` (or `turn_aborted`); an approval or question request
that is the last thing in the log means it is waiting (best effort: not every setup records those).
"""

from __future__ import annotations

import json
import re
from pathlib import Path

from geekmagic.adapters.providers.logs import TOOL_MAX_AGE, TURN_MAX_AGE, WAIT_MAX_AGE, collect, epoch, scan_state, tail_lines
from geekmagic.core.activity import Event, project_label, summarize

CODEX_DIR = Path.home() / ".codex" / "sessions"


_TIMESTAMP_RE = re.compile(r'"timestamp"\s*:\s*"([^"]+)"')


def read_events(path: Path) -> list[Event]:
    """Every model call in one Codex session log: a `token_count` event that carries token info. Which project and
    model it belongs to is whatever the latest `turn_context` / `session_meta` before it said."""
    found = []
    project, model = "(unknown)", "(unknown)"
    with path.open(encoding="utf-8", errors="replace") as f:
        for line in f:
            if '"turn_context"' in line or '"session_meta"' in line:
                try:
                    payload = json.loads(line).get("payload") or {}
                except ValueError:
                    continue
                if isinstance(payload, dict):
                    project = project_label(payload.get("cwd")) if payload.get("cwd") else project
                    model = payload.get("model") or model
            elif '"token_count"' in line and '"info":null' not in line.replace(" ", ""):
                match = _TIMESTAMP_RE.search(line)
                when = epoch(match.group(1)) if match else None
                if when is not None:
                    found.append((when, path.stem, None, project, model))
    return found


def stats(now_epoch: float, root: Path | None = None) -> dict | None:
    root = root or CODEX_DIR
    return summarize(collect(root, read_events, now_epoch), now_epoch) if root.is_dir() else None


CODEX_REQUESTS = ("approval_request", "request_user_input", "elicitation_request")  # Codex events that wait on you


_PAYLOAD_TYPE_RE = re.compile(r'"payload"\s*:\s*\{\s*"type"\s*:\s*"([A-Za-z_]+)"')


def _summary(path: Path) -> dict | None:
    marker, marker_time, last_time, last_type, project = None, None, None, None, None
    for line in tail_lines(path):
        found = next((k for k in ("task_started", "task_complete", "turn_aborted") if f'"{k}"' in line), None)
        if '"turn_context"' in line:
            try:
                cwd = (json.loads(line).get("payload") or {}).get("cwd")
                project = project_label(cwd) if cwd else project
            except ValueError:
                pass
        match = _TIMESTAMP_RE.search(line)
        when = epoch(match.group(1)) if match else None
        if when is not None:
            last_time = when
        typed = _PAYLOAD_TYPE_RE.search(line)
        last_type = typed.group(1) if typed else last_type
        if found:
            marker, marker_time = found, when
    if last_time is None:
        return None
    return {"marker": marker, "marker_time": marker_time, "last": last_time, "last_type": last_type, "project": project,
            "session": path.stem}


def _decide(summary: dict | None, now_epoch: float) -> dict | None:
    if not summary:
        return None
    age = now_epoch - summary["last"]
    if summary["marker"] is None:  # a long run whose markers have scrolled out of the tail: judge by recent writes
        working = age <= TURN_MAX_AGE
        return {"id": summary["session"], "working": working, "waiting": False, "waiting_for": None, "since": None,
                "last": summary["last"], "project": summary["project"]}
    in_turn = summary["marker"] == "task_started"
    asking = in_turn and age <= WAIT_MAX_AGE and any(k in (summary["last_type"] or "") for k in CODEX_REQUESTS)
    working = in_turn and (asking or age <= TOOL_MAX_AGE)
    return {"id": summary["session"], "working": working, "waiting": asking, "waiting_for": "approval" if asking else None,
            "since": summary["marker_time"] if working else None, "last": summary["last"], "project": summary["project"]}


def agent_state(now_epoch: float, root: Path | None = None) -> dict:
    return scan_state(root or CODEX_DIR, _summary, _decide, now_epoch)
