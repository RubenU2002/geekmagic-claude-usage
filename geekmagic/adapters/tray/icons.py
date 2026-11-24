"""The tray icon: the mascot of the provider on screen, or a picture of the view of two that is."""

from __future__ import annotations

from PIL import Image, ImageDraw

from geekmagic.core.themes import BG, THEMES, blend
from geekmagic.core.views import BREAKDOWN, HOURS, SPLIT, STATS


def _mascot_image(title: str) -> Image.Image:
    theme = THEMES[title]
    image = Image.new("RGBA", (42, 24), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    for row, line in enumerate(theme["bitmap"]):
        for col, cell in enumerate(line):
            if cell != "0":
                fill = theme["body"] if cell == "1" else "#0F0D0B"
                draw.rectangle((col * 3, row * 3, col * 3 + 2, row * 3 + 2), fill=fill)
    return image


def make_icon(title: str) -> Image.Image:
    """Mascot on a transparent background, tinted per provider so you can tell them apart."""
    canvas = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    canvas.paste(_mascot_image(title).resize((60, 34), Image.NEAREST), (2, 15))
    return canvas


def make_split_icon() -> Image.Image:
    """Both mascots stacked: the icon shown while the split view is on."""
    canvas = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    canvas.paste(_mascot_image("Claude"), (11, 4))
    canvas.paste(_mascot_image("Codex"), (11, 36))
    return canvas


def make_stats_icon() -> Image.Image:
    """Three bars in the providers' colours: the icon shown while the stats view is on."""
    canvas = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    draw = ImageDraw.Draw(canvas)
    for i, (height, color) in enumerate(((26, THEMES["Claude"]["body"]), (40, THEMES["Codex"]["body"]), (18, THEMES["Codex"]["weekly"]))):
        x = 10 + i * 17
        draw.rectangle((x, 54 - height, x + 12, 54), fill=color)
    return canvas


def make_breakdown_icon() -> Image.Image:
    """Three horizontal bars of different lengths: shares of a total."""
    canvas = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    draw = ImageDraw.Draw(canvas)
    for i, (length, color) in enumerate(((46, THEMES["Claude"]["body"]), (32, THEMES["Codex"]["body"]), (18, THEMES["Codex"]["weekly"]))):
        y = 12 + i * 15
        draw.rectangle((8, y, 8 + length, y + 9), fill=color)
    return canvas


def make_hours_icon() -> Image.Image:
    """A little histogram with its busy stretch in the middle: the hours of the day."""
    canvas = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    draw = ImageDraw.Draw(canvas)
    for i, height in enumerate((8, 12, 22, 38, 44, 30, 16, 10)):
        x = 6 + i * 7
        draw.rectangle((x, 54 - height, x + 5, 54), fill=THEMES["Claude"]["body"] if 3 <= i <= 4 else blend(THEMES["Claude"]["body"], BG, 0.45))
    return canvas


def icon_for(key: str, titles: dict[str, str]) -> Image.Image:
    """The icon of a view: a provider's key (`titles` says what it is called), or one of the views of two."""
    return {SPLIT: make_split_icon, STATS: make_stats_icon, BREAKDOWN: make_breakdown_icon,
            HOURS: make_hours_icon}.get(key, lambda: make_icon(titles[key]))()
