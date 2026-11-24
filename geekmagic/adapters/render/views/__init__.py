"""The screens, one class each. A view turns the readings it is given into the GIF to upload.

Each says what it needs (`panels`: a single provider's reading, or the readings of two) and returns a `Rendered`: the GIF
plus which rotation picks it made, so the caller can remember them once the upload has gone through.
"""

from __future__ import annotations

from abc import ABC, abstractmethod

from geekmagic.core.model import ErrorScreen, Usage
from geekmagic.core.ports import Rendered
from geekmagic.adapters.render.animations import ANIMATIONS, IDLE_BOB, Rotation, animation_for, busy_animation
from geekmagic.adapters.render.gif import encode_gif
from geekmagic.adapters.render.views import error, panels as panel_views, single, split


class View(ABC):
    key: str
    needs_activity: bool = False  # drawn from the providers' local activity logs (they're counted in the background)

    @abstractmethod
    def render(self, data, animation: str, rotation: Rotation) -> Rendered:
        """`data` is one reading (a single provider's screen) or a list of two (the views of both)."""


class SingleView(View):
    """One provider's screen, with its mascot playing an animation."""

    def render(self, usage: Usage, animation: str, rotation: Rotation) -> Rendered:
        rotating = animation in ("auto", "random")
        if rotating and busy_animation(usage):  # its agent is busy: the mascot shows it (and that isn't a rotation pick)
            animation, rotating = busy_animation(usage), False
        elif rotating:
            animation = rotation.pick(usage.title)
        gif = single.render_animation(usage, animation)
        picks = [(usage.title, animation)] if rotating else []
        return Rendered(gif, [animation], picks)


class ErrorView(View):
    """A provider that can't be read: its mascot and why."""

    def render(self, screen: ErrorScreen, animation: str = "idle", rotation: Rotation | None = None) -> Rendered:
        return Rendered(encode_gif([error.render_error_frame(screen)]))


class SplitView(View):
    key = "split"

    def render(self, panels: list[Usage], animation: str, rotation: Rotation) -> Rendered:
        """Each provider gets a fresh animation of its own ("auto" / "random"), remembered once uploaded."""
        names = [animation_for(usage, animation, rotation) for usage in panels[:2]]
        picks = []
        if animation in ("auto", "random"):
            picks = [(usage.title, name) for usage, name in zip(panels[:2], names)
                     if not usage.working]  # busy agent: its mascot works, which isn't a rotation pick
        return Rendered(split.render_split(panels, names), names, picks)


class _StillPanels(View):
    """The views of both providers' activity: a single still frame (a small upload)."""

    needs_activity = True
    frame = staticmethod(lambda panels: None)

    def render(self, panels: list[Usage], animation: str = "idle", rotation: Rotation | None = None) -> Rendered:
        return Rendered(encode_gif([self.frame(panels)]))


class StatsView(_StillPanels):
    key = "stats"
    frame = staticmethod(panel_views.render_stats_frame)


class BreakdownView(_StillPanels):
    key = "breakdown"
    frame = staticmethod(panel_views.render_breakdown_frame)


class HoursView(_StillPanels):
    key = "hours"
    frame = staticmethod(panel_views.render_hours_frame)


SINGLE = SingleView()
ERROR = ErrorView()
PANEL_VIEWS: dict[str, View] = {v.key: v for v in (SplitView(), StatsView(), BreakdownView(), HoursView())}

__all__ = ["ANIMATIONS", "ERROR", "IDLE_BOB", "PANEL_VIEWS", "Rendered", "SINGLE", "View"]
