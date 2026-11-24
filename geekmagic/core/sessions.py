"""Several sessions of one agent at once (you may be working in several projects): merging and describing them."""

from __future__ import annotations

def merge_sessions(states: list[dict]) -> list[dict]:
    """One entry per session (a session's sub-agent logs are merged into it): working if any of its logs is, waiting
    if any is, the project of its freshest log. This is what lets several projects be followed at the same time."""
    merged: dict[str, dict] = {}
    for state in sorted(states, key=lambda s: s["last"]):  # oldest first, so the freshest log's project wins
        mine = merged.setdefault(state["id"], {**state, "working": False, "waiting": False, "waiting_for": None, "since": None})
        mine["last"] = max(mine["last"], state["last"])
        mine["project"] = state["project"] or mine["project"]
        if state["working"]:
            mine["working"] = True
            mine["since"] = min(filter(None, (mine["since"], state["since"])), default=None)
        if state["waiting"]:
            mine["waiting"], mine["waiting_for"] = True, state["waiting_for"]
    return list(merged.values())


def duration_text(seconds: float) -> str:
    """"45 s", "4 min", "1 h 05 min"."""
    seconds = max(0, round(seconds))
    if seconds < 60:
        return f"{seconds} s"
    minutes = seconds // 60
    return f"{minutes} min" if minutes < 60 else f"{minutes // 60} h {minutes % 60:02d} min"
