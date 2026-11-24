"""The names of the image files kept on the device."""

from __future__ import annotations

LEGACY_NAMES = ("claude-usage.jpg",)  # from before the GIF switch; cleaned up once


def image_name(key: str) -> str:
    """The one file of a provider the command-line script writes (the tray app alternates between two, see slot_file)."""
    return f"{key}-usage.gif"


def slot_file(key: str, slot: str) -> str:
    """One of the two files a view alternates between (slot "a" / "b"): an upload cut short by a click can only damage
    the spare one, so the one on screen (and the one a switch will show) stays intact."""
    return f"{key}-usage-{slot}.gif"
