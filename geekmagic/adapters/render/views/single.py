"""One provider's screen: mascot, two bars, reset countdowns, pace."""

from __future__ import annotations

from io import BytesIO

from PIL import Image, ImageDraw, ImageFont

from geekmagic.core.model import Usage
from geekmagic.adapters.render.animations import ANIMATIONS, DEFAULT_ANIMATION
from geekmagic.adapters.render.components import busy_style, clock, draw_section, draw_working_dot, format_delta, resets_chip, window_pace
from geekmagic.adapters.render.gif import encode_gif
from geekmagic.adapters.render.mascots import MASCOT_TOP, draw_mascot, theme_for
from geekmagic.adapters.render.palette import BG, HEIGHT, STATE_COLORS, TEXT, WIDTH


def render_frame(usage: Usage, mascot_dx: int = 0, mascot_dy: int = 0, extra=None) -> Image.Image:
    image = Image.new("RGB", (WIDTH, HEIGHT), BG)
    draw = ImageDraw.Draw(image)

    title_font = ImageFont.load_default(size=26)
    pill_font = ImageFont.load_default(size=14)
    pct_font = ImageFont.load_default(size=30)
    reset_font = ImageFont.load_default(size=14)
    footer_font = ImageFont.load_default(size=13)

    theme = theme_for(usage)
    mascot_cell = 3
    mascot_w = mascot_cell * len(theme["bitmap"][0])
    draw_mascot(image, (10 + mascot_dx, MASCOT_TOP + mascot_dy), mascot_cell, theme)

    if extra is not None:
        extra(image, mascot_w)

    draw.text((10 + mascot_w + 10, 8), usage.title, font=title_font, fill=TEXT)
    busy = busy_style(usage)  # its agent is running (green "Working") or waiting for you (amber "Waiting"), top right
    working = busy is not None
    if busy:
        label, color = busy
        label_w = draw.textlength(label, font=footer_font)
        draw.text((230 - label_w, 7), label, font=footer_font, fill=color)
        draw_working_dot(draw, 230 - label_w - 11, 15, color)
    chip = resets_chip(usage)  # Codex only: the free rate-limit resets you still have, top right
    if chip:
        text, color = chip
        draw.text((230 - draw.textlength(text, font=footer_font), 23 if working else 15), text, font=footer_font, fill=color)

    draw_section(
        draw, top=40, label="Session", percent=usage.current.pct,
        reset_text=format_delta(usage.current.reset, usage.now),
        accent=theme["current"],
        pill_font=pill_font, pct_font=pct_font, reset_font=reset_font,
        pace_info=window_pace(usage, "current"), pace_font=footer_font,
    )
    draw_section(
        draw, top=134, label="Weekly", percent=usage.weekly.pct,
        reset_text=format_delta(usage.weekly.reset, usage.now),
        accent=theme["weekly"],
        pill_font=pill_font, pct_font=pct_font, reset_font=reset_font,
        pace_info=window_pace(usage, "weekly"), pace_font=footer_font,
    )

    if usage.stale:
        # Last known numbers that couldn't be refreshed: dim everything and say since when.
        image = Image.blend(image, Image.new("RGB", image.size, BG), 0.55)
        draw = ImageDraw.Draw(image)
        footer_text, footer_color = f"! No fresh data since {clock(usage.now)}", STATE_COLORS["warn"]
    else:
        footer_text, footer_color = f"* Updated {clock(usage.now)}", theme["current"]
    bbox = draw.textbbox((0, 0), footer_text, font=footer_font)
    draw.text((((240 - (bbox[2] - bbox[0])) // 2), 223), footer_text, font=footer_font, fill=footer_color)

    return image


def render(usage: Usage) -> bytes:
    """Static JPEG (single frame, mascot at rest)."""
    buf = BytesIO()
    render_frame(usage).save(buf, format="JPEG", quality=92)
    return buf.getvalue()


def render_animation(usage: Usage, animation: str = DEFAULT_ANIMATION) -> bytes:
    """Looping GIF for one of the named ANIMATIONS presets."""
    frames_spec = ANIMATIONS[animation]
    return encode_gif([render_frame(usage, dx, dy, extra) for dx, dy, extra in frames_spec])
