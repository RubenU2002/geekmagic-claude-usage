"""The Renderer port, drawn with Pillow: readings in, a GIF out."""

from __future__ import annotations

from io import BytesIO

from PIL import Image

from geekmagic.adapters.render import views
from geekmagic.adapters.render.animations import Rotation
from geekmagic.adapters.render.version import RENDER_ID
from geekmagic.core.model import ErrorScreen, Usage
from geekmagic.core.ports import Rendered
from geekmagic.core.views import PANEL_KEYS


class PillowRenderer:
    version = RENDER_ID

    def __init__(self) -> None:
        self.rotation = Rotation()  # which animation each provider played lately, so none comes back too soon
        missing = set(PANEL_KEYS) - set(views.PANEL_VIEWS)
        assert not missing, f"no view draws {missing}"

    def render(self, key: str, data: Usage | list[Usage] | ErrorScreen, animation: str) -> Rendered:
        if key in views.PANEL_VIEWS:
            return views.PANEL_VIEWS[key].render(data, animation, self.rotation)
        if isinstance(data, ErrorScreen):
            return views.ERROR.render(data)
        return views.SINGLE.render(data, animation, self.rotation)

    def to_png(self, gif: bytes) -> bytes:
        with Image.open(BytesIO(gif)) as image:
            image.seek(0)  # the whole picture, the way the screen first shows it (the later frames only move the mascot)
            out = BytesIO()
            image.convert("RGB").save(out, format="PNG")
        return out.getvalue()

    def confirm(self, rendered: Rendered) -> None:
        for title, name in rendered.picks:
            self.rotation.record(title, name)  # now it really is on the device

    def restore(self, saved: dict) -> None:
        self.rotation.restore(saved)

    def snapshot(self) -> dict:
        return self.rotation.snapshot()
