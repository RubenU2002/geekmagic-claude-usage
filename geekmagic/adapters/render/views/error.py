"""The screen of a provider that can't be read yet: its mascot and why."""

from __future__ import annotations

from PIL import Image, ImageDraw, ImageFont

from geekmagic.core.model import ErrorScreen
from geekmagic.adapters.render.mascots import MASCOT_TOP, draw_mascot, theme_for
from geekmagic.adapters.render.palette import BG, HEIGHT, MUTED, STATE_COLORS, TEXT, WIDTH


def render_error_frame(usage: ErrorScreen) -> Image.Image:
    """What a provider's screen says when it has never been read: its mascot and why (not signed in, not installed...).
    Without it a click on a provider that can't be read would change nothing at all and look broken."""
    image = Image.new("RGB", (WIDTH, HEIGHT), BG)
    draw = ImageDraw.Draw(image)
    theme = theme_for(usage)
    mascot_w = 3 * len(theme["bitmap"][0])
    draw_mascot(image, (10, MASCOT_TOP), 3, theme)
    draw.text((10 + mascot_w + 10, 8), usage.title, font=ImageFont.load_default(size=26), fill=TEXT)
    heading, body = ImageFont.load_default(size=17), ImageFont.load_default(size=15)
    draw.text((10, 56), "Sign-in needed" if usage.signin else "Can't read the usage", font=heading, fill=STATE_COLORS["warn"])
    # the reason, word-wrapped to the screen (what a provider's own error says is meant to be read by a person)
    lines, line = [], ""
    for word in (usage.message or "No reading yet.").split():
        trial = f"{line} {word}".strip()
        if draw.textlength(trial, font=body) <= WIDTH - 20:
            line = trial
        else:
            lines.append(line)
            line = word
    lines.append(line)
    for i, text in enumerate(lines[:7]):
        draw.text((10, 86 + i * 20), text.strip(), font=body, fill=TEXT if i < 6 else MUTED)
    footer = "Retrying every few minutes"
    draw.text(((WIDTH - draw.textlength(footer, font=ImageFont.load_default(size=13))) // 2, 223), footer,
              font=ImageFont.load_default(size=13), fill=MUTED)
    return image
