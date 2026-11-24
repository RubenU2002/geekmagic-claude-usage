"""Running something off the caller's thread."""

import threading


def in_background(func, *args) -> None:
    threading.Thread(target=func, args=args, daemon=True).start()
