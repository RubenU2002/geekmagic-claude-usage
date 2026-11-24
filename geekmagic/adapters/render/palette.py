"""Colours and sizes shared by every screen."""

from __future__ import annotations

# the colours that belong to a provider's look live in core.themes (the tray icon uses them too); re-exported here
from geekmagic.core.themes import ACCENT, BG, CURRENT_ACCENT, WEEKLY_ACCENT, blend, hex_rgb  # noqa: F401


WIDTH = HEIGHT = 240
SPLIT_PANEL_H = 118  # two panels of this height, a 4 px divider between them


PILL_BG = "#4A3F4D"


# Past the thresholds in alerts.py the percentage and its bar swap the provider's accent for these.
STATE_COLORS = {"warn": "#FFD23F", "crit": "#FF2D55"}


TEXT = "#F5F4EF"


MUTED = "#C7C2BC"


WORKING_COLOR = "#4ADE80"  # the dot (and word) that says an agent is running right now


WAITING_COLOR = "#FFB020"  # ...and amber when it has stopped and is waiting for you
