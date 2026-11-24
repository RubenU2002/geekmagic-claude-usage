"""Each provider's pixel-art mascot and colours."""

from __future__ import annotations

from PIL import Image, ImageDraw

from geekmagic.core.model import Usage
from geekmagic.core.themes import CLAUDE_BITMAP, CODEX_BITMAP, INK, THEMES  # noqa: F401  (re-exported)


def theme_for(usage: Usage) -> dict:
    return THEMES.get(usage.title, THEMES["Claude"])


def draw_mascot(image: Image.Image, top_left: tuple[int, int], cell_size: int, theme: dict | None = None) -> None:
    """Blocky pixel-art mascot, drawn cell by cell so it stays crisp at small sizes."""
    theme = theme or THEMES["Claude"]
    x0, y0 = top_left
    draw = ImageDraw.Draw(image)
    for row, line in enumerate(theme["bitmap"]):
        for col, cell in enumerate(line):
            if cell == "0":
                continue
            fill = theme["body"] if cell == "1" else INK
            x = x0 + col * cell_size
            y = y0 + row * cell_size
            draw.rectangle((x, y, x + cell_size - 1, y + cell_size - 1), fill=fill)


MASCOT_TOP = 10
