"""The simulated screen: a window that plays what the GeekMagic would be showing, for when it isn't there.

It runs Tk on a thread of its own (the tray owns the main one) and is built on demand: closing it frees everything. All the
Tk calls happen on that thread; the rest of the app talks to it through a queue.
"""

from __future__ import annotations

import gc
import logging
import queue
import subprocess
import sys
import tempfile
import threading
from collections.abc import Callable
from io import BytesIO
from pathlib import Path

from PIL import Image, ImageSequence

from geekmagic.core.themes import BG

log = logging.getLogger("tray")

SIDE = 240  # the real screen is 240 x 240 pixels
SCALE = 1  # ...and drawn at that size: pixel for pixel what the real one shows
FRAME_MS = 130  # what the GIFs ask for between frames
POLL_MS = 100  # how often a still picture looks for requests and a new picture


def decode_frames(gif: bytes) -> list[Image.Image]:
    """Every frame of a GIF as a complete picture (the later frames are stored as just what changed)."""
    with Image.open(BytesIO(gif)) as image:
        return [frame.convert("RGB") for frame in ImageSequence.Iterator(image)]


class SimulatorWindow:
    """`latest()` is the (view, version, GIF) to show (None while there is nothing to show), `on_click()` what a click in the
    window does, `title()` what its title bar says."""

    def __init__(self, latest: Callable[[], tuple[str, int, bytes] | None], on_click: Callable[[], None],
                 title: Callable[[], str]) -> None:
        self.latest = latest
        self.on_click = on_click
        self.title = title
        self._requests: queue.Queue[str] = queue.Queue()
        self._thread: threading.Thread | None = None
        self._lock = threading.Lock()

    @property
    def is_open(self) -> bool:
        return self._thread is not None and self._thread.is_alive()

    def show(self) -> None:
        """Open the window, or bring it to the front."""
        if sys.platform == "darwin":
            return self._open_in_viewer()
        self._start()
        self._requests.put("show")

    def toggle(self) -> None:
        """Open the window, or close it if it is open."""
        if sys.platform == "darwin":
            return self._open_in_viewer()
        if self.is_open:
            self._requests.put("close")
        else:
            self._start()

    def close(self) -> None:
        if self.is_open:
            self._requests.put("close")

    def _start(self) -> None:
        with self._lock:
            if not self.is_open:
                self._requests = queue.Queue()
                self._thread = threading.Thread(target=self._run, name="simulated-screen", daemon=True)
                self._thread.start()

    def _open_in_viewer(self) -> None:
        """macOS: Tk wants the main thread there, which the tray owns, so the picture opens in the system's viewer."""
        found = self.latest()
        if found is None:
            return
        path = Path(tempfile.gettempdir()) / "geekmagic-simulated-screen.gif"
        path.write_bytes(found[2])
        subprocess.Popen(["open", str(path)])

    def _run(self) -> None:
        try:
            self._serve()
        finally:
            # Tk objects must be freed on the thread that made them (Tcl aborts the whole process otherwise). The window's
            # callbacks and pictures form reference cycles, which only the garbage collector frees: do it here, now that
            # _serve has returned and let go of them, not later on whatever thread happens to be running when it fires.
            gc.collect()

    def _serve(self) -> None:
        try:
            import tkinter as tk
            from PIL import ImageTk
            root = tk.Tk()
        except Exception:  # no desktop, or Tk isn't installed
            log.warning("could not open the simulated screen", exc_info=True)
            return
        side = SIDE * SCALE
        root.title(self.title())
        root.configure(bg=BG)
        root.resizable(False, False)
        canvas = tk.Canvas(root, width=side, height=side, bg=BG, highlightthickness=0, cursor="hand2")
        canvas.pack()
        waiting = canvas.create_text(side // 2, side // 2, text="Esperando datos...", fill="#C7C2BC", font=("Segoe UI", 14))
        picture = canvas.create_image(0, 0, anchor="nw")
        state = {"shown": None, "frames": [], "index": 0, "photo": None}

        def clicked(_event=None) -> None:
            threading.Thread(target=self.on_click, daemon=True).start()

        canvas.bind("<Button-1>", clicked)
        root.protocol("WM_DELETE_WINDOW", root.destroy)

        def draw(index: int) -> None:
            frame = state["frames"][index]
            if SCALE != 1:
                frame = frame.resize((side, side), Image.NEAREST)
            state["photo"] = ImageTk.PhotoImage(frame)  # (kept: Tk shows nothing of a picture that was garbage collected)
            canvas.itemconfigure(picture, image=state["photo"])

        def tick() -> None:
            while True:
                try:
                    request = self._requests.get_nowait()
                except queue.Empty:
                    break
                if request == "close":
                    root.destroy()
                    return
                if request == "click":
                    clicked()
                elif request == "show":
                    root.deiconify()
                    root.lift()
                    root.attributes("-topmost", True)  # (to the front once; it is not kept there)
                    root.after(200, lambda: root.attributes("-topmost", False))
            found = self.latest()
            if found is not None and (found[0], found[1]) != state["shown"]:
                try:
                    state.update(frames=decode_frames(found[2]), index=0, shown=(found[0], found[1]))
                    canvas.itemconfigure(waiting, state="hidden")
                    draw(0)
                except Exception:
                    log.warning("could not show the picture of %s", found[0], exc_info=True)
                    state["shown"] = (found[0], found[1])
            elif len(state["frames"]) > 1:
                state["index"] = (state["index"] + 1) % len(state["frames"])
                draw(state["index"])
            root.title(self.title())
            root.after(FRAME_MS if len(state["frames"]) > 1 else POLL_MS, tick)

        tick()
        try:
            root.mainloop()
        except Exception:
            log.warning("the simulated screen stopped", exc_info=True)
        finally:
            state.clear()  # (the pictures)
            try:
                root.destroy()
            except tk.TclError:
                pass  # already gone: it was closed

