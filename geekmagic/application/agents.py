"""Is Claude / Codex working right now, waiting for you, or done? Followed one session at a time, since you may be
working in several projects at once and each one's run starts, asks you something and finishes on its own."""

from __future__ import annotations

import logging
import threading
import time
from collections.abc import Callable
from dataclasses import replace

from geekmagic.application import config
from geekmagic.core.sessions import duration_text
from geekmagic.core.model import ErrorScreen
from geekmagic.core.registry import ProviderRegistry
from geekmagic.core.views import PANEL_KEYS

log = logging.getLogger("tray")


class AgentMonitor:
    """Polls the providers' logs and turns what they say into per-session tracks.

    The app hooks in through `on_change()` (an agent started or stopped: redraw) and `notify(title, message)`.
    """

    def __init__(self, providers: ProviderRegistry, notify: Callable[[str, str], None], on_change: Callable[[], None],
                 stop: threading.Event, events_enabled: Callable[[], bool]) -> None:
        self.providers = providers
        self.notify = notify
        self.on_change = on_change
        self.stop = stop
        self.events_enabled = events_enabled  # is the "tell me when an agent finishes or waits" switch on
        self.agent: dict[str, dict] = {}  # provider -> latest summary (is it working right now?)
        # provider -> session id -> what's known of that session (see track_session): one per project you work in
        self.tracks: dict[str, dict[str, dict]] = {}
        self.dirty = False  # an agent started or stopped: redraw from what's already read

    # --- what the screens ask ---------------------------------------------------------------------------------------

    def working(self, provider: str | None) -> bool:
        return bool(provider and self.agent.get(provider, {}).get("working"))

    def waiting(self, provider: str | None) -> bool:
        return bool(provider and self.agent.get(provider, {}).get("waiting"))

    def flag(self, key: str, data):
        """What's about to be drawn, with each provider's "its agent is working right now" flag set from the latest
        look at its logs (the screen draws a tag and the mascot works). `key` is a provider's (`data` is its Usage, or
        an ErrorScreen), or one of the views of two (`data` is a list of Usage)."""
        if isinstance(data, ErrorScreen):
            return data
        if key in self.providers:
            return replace(data, working=self.working(key), waiting=self.waiting(key))
        if key in PANEL_KEYS:
            return [replace(p, working=self.working(self.providers.key_of(p.title)),
                            waiting=self.waiting(self.providers.key_of(p.title))) for p in data]
        return data

    # --- polling ----------------------------------------------------------------------------------------------------

    def watch(self) -> None:
        """Thread: look at the agents' logs every few seconds, even while paused or locked (that's when you want
        to hear that one finished)."""
        while not self.stop.is_set():
            self.poll()
            self.stop.wait(config.AGENT_POLL)

    def poll(self) -> None:
        for provider in self.providers.values():
            try:
                state = provider.agent_state(time.time())
            except Exception:
                log.warning("could not read %s's activity", provider.key, exc_info=True)
                continue
            self.update(provider.key, state)

    def update(self, provider: str, state: dict) -> None:
        """Follow a provider's agents, one session at a time. Without a list of sessions the whole state is treated as
        a single one. The screens only need the sum: is any of them working / waiting?"""
        sessions = state["sessions"] if "sessions" in state else [{**state, "id": "-"}]  # (an empty list is "none")
        tracks = self.tracks.setdefault(provider, {})
        seen = {s.get("id") or "-" for s in sessions}
        changed = False
        for s in sessions:
            changed |= self.track_session(provider, s.get("id") or "-", s, tracks)
        for gone in [sid for sid in tracks if sid not in seen]:  # its log went quiet for good: it can only be idle
            changed |= self.track_session(provider, gone, {"working": False, "waiting": False}, tracks)
        for idle in [sid for sid, t in tracks.items() if not t["working"]]:
            del tracks[idle]  # an idle session needs no memory: the debounce counters only matter while it works
        busy = [t for t in tracks.values() if t["working"]]
        waiting = [t for t in busy if t["waiting"]]
        lead = (waiting or busy or [{}])[0]  # the one that needs you, else any that works (for the screens' summary)
        self.agent[provider] = {
            "working": bool(busy), "waiting": bool(waiting), "waiting_for": lead.get("waiting_for") if waiting else None,
            "project": lead.get("project") if busy else state.get("project"), "since": min((t["started"] for t in busy), default=None),
            "sessions": len(busy),
        }
        if changed:
            self.dirty = True
            self.on_change()

    def track_session(self, provider: str, sid: str, s: dict, tracks: dict) -> bool:
        """One session: a run starting shows at once; a question is believed after AGENT_WAIT_POLLS polls in a row and
        a run ending after AGENT_IDLE_POLLS quiet ones, so the gaps between its steps don't flicker or notify early.
        Returns whether something visible changed."""
        t = tracks.setdefault(sid, {"working": False, "waiting": False, "waiting_for": None, "started": None,
                                    "idle_polls": 0, "wait_polls": 0, "project": None})
        if s.get("project"):
            t["project"] = s["project"]
        changed = False
        if s.get("working"):
            t["idle_polls"] = 0
            if not t["working"]:
                t.update(working=True, started=s.get("since") or time.time())
                log.info("%s started working (%s)", provider, t["project"])
                changed = True
            t["wait_polls"] = t["wait_polls"] + 1 if s.get("waiting") else 0
            waiting_now = bool(s.get("waiting")) and (t["waiting"] or t["wait_polls"] >= config.AGENT_WAIT_POLLS)
            if waiting_now != t["waiting"]:
                t["waiting"], t["waiting_for"] = waiting_now, (s.get("waiting_for") if waiting_now else None)
                log.info("%s (%s) %s", provider, t["project"],
                         f"is waiting for you ({t['waiting_for']})" if waiting_now else "is no longer waiting for you")
                if waiting_now:
                    self.notify_waiting(provider, t)
                changed = True
        elif t["working"]:
            t["idle_polls"] += 1
            if t["idle_polls"] >= config.AGENT_IDLE_POLLS:
                lasted = time.time() - t["started"] if t["started"] else 0
                log.info("%s (%s) finished (worked %s)", provider, t["project"], duration_text(lasted))
                if self.events_enabled() and lasted >= config.AGENT_MIN_RUN:
                    self.notify(self.title(provider, t["project"]), f"Terminó (trabajó {duration_text(lasted)})")
                t.update(working=False, waiting=False, waiting_for=None, started=None, wait_polls=0)
                changed = True
        return changed

    # --- what it says -----------------------------------------------------------------------------------------------

    def title(self, provider: str, project: str | None) -> str:
        """"Claude · Acme App": the project is in the title, which is what you read first when you have several going."""
        name = self.providers.titles[provider]
        return f"{name} · {project}" if project else name

    def notify_waiting(self, provider: str, session: dict) -> None:
        """It stopped and needs you: say what for (a question, a plan to approve, a tool to allow); the title says where."""
        reason = session.get("waiting_for") or ""
        if reason == "question":
            what = "Te hizo una pregunta"
        elif reason == "plan":
            what = "Espera que apruebes su plan"
        elif reason.startswith("approval"):
            what = "Espera tu aprobación" + reason[len("approval"):]  # "approval (Edit)" -> "...aprobación (Edit)"
        else:
            what = "Espera tu respuesta"
        if self.events_enabled():
            self.notify(self.title(provider, session.get("project")), what)

    def tooltip_notes(self) -> list[str]:
        titles = self.providers.titles
        return [f"{titles[p]} te espera" if self.waiting(p) else f"{titles[p]} trabajando" for p in titles if self.working(p)]

