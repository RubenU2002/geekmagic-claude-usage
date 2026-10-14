"""Taking a picture of what the screen is showing, and saving it where you say."""

from __future__ import annotations

import logging
from collections.abc import Callable
from datetime import datetime
from pathlib import Path

from geekmagic.application import image_files
from geekmagic.application.screen import Screen
from geekmagic.application.threads import in_background
from geekmagic.core.ports import Renderer

log = logging.getLogger("tray")


class ScreenCapture:
    def __init__(self, screen: Screen, renderer: Renderer, ask_save_path: Callable[[str], str | None],
                 notify: Callable[[str, str], None]) -> None:
        self.screen = screen
        self.renderer = renderer
        self.ask_save_path = ask_save_path
        self.notify = notify

    def take(self) -> None:
        """The menu's command: it asks where to save, which takes as long as you take, so it runs off the caller's thread."""
        in_background(self.capture)

    def capture(self) -> Path | None:
        """Save the image on the screen (it is read back from the device, so it is what is really there) as a PNG, or as
        the animated GIF itself when the name you choose ends in .gif. Returns the file, None if nothing was saved."""
        s = self.screen
        key = s.active_key()
        slot = s.slots.get(key)
        if not s.ip or s.offline:
            return self._failed("la pantalla no responde ahora")
        if slot is None:
            return self._failed("todavía no hay una imagen en la pantalla")
        stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
        chosen = self.ask_save_path(f"geekmagic-{key}-{stamp}.png")
        if not chosen:
            return None
        path = Path(chosen)
        try:
            with s.push_lock:  # the device serves one request at a time
                gif = s.device.download_image(image_files.slot_file(key, slot))
            path.write_bytes(gif if path.suffix.lower() == ".gif" else self.renderer.to_png(gif))
        except Exception as e:
            log.warning("could not save the screenshot: %s", e, exc_info=not isinstance(e, OSError))
            return self._failed(str(e))
        log.info("screenshot saved: %s", path)
        self.notify("Pantallazo guardado", str(path))
        return path

    def _failed(self, why: str) -> None:
        self.notify("Pantallazo", f"No se pudo guardar: {why}")
        return None
