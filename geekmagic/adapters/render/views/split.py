"""The split view: two providers on one screen, each mascot doing its own animation."""

from __future__ import annotations

from PIL import Image, ImageDraw, ImageFont

from geekmagic.core.model import Usage
from geekmagic.adapters.render.animations import ANIMATIONS, IDLE_BOB
from geekmagic.adapters.render.components import clock, draw_busy_tag, draw_split_row, format_delta, window_pace
from geekmagic.adapters.render.gif import encode_gif
from geekmagic.adapters.render.mascots import MASCOT_TOP, draw_mascot, theme_for
from geekmagic.adapters.render.palette import BG, HEIGHT, PILL_BG, SPLIT_PANEL_H, STATE_COLORS, TEXT, WIDTH


# --- Split view: both providers on one screen ---------------------------------------------------------



SPLIT_ART = (64, 40)  # the mascot-and-props corner of a split panel: the same art, at the same scale, as the single screens


def header_art(usage: Usage, frame: tuple) -> Image.Image:
    """The top-left corner of a single-provider screen for one animation frame (mascot at full size plus whatever it
    holds: laptop, mug, bolt, sparkles...). The animations draw at absolute coordinates, so draw the real thing on a
    scratch screen and cut the corner out."""
    scratch = Image.new("RGB", (WIDTH, HEIGHT), BG)
    dx, dy, extra = frame
    theme = theme_for(usage)
    draw_mascot(scratch, (10 + dx, MASCOT_TOP + dy), 3, theme)
    if extra is not None:
        extra(scratch, 3 * len(theme["bitmap"][0]))
    return scratch.crop((0, 0, *SPLIT_ART))


def draw_split_panel(image: Image.Image, top: int, usage: Usage, frame: tuple, fonts: dict) -> None:
    """One provider's panel (animated header + two rows) at vertical offset `top`; dimmed and dated if stale."""
    theme = theme_for(usage)
    draw = ImageDraw.Draw(image)
    image.paste(header_art(usage, (0, 0, None) if usage.stale else frame), (0, top))
    draw.text((SPLIT_ART[0] + 2, top + 8), usage.title, font=fonts["title"], fill=TEXT)
    updated = clock(usage.now)
    if not usage.stale:
        width = draw.textlength(updated, font=fonts["small"])
        draw.text((230 - width, top + 12), updated, font=fonts["small"], fill=theme["current"])
        draw_busy_tag(draw, top + 25, usage, fonts["small"])
    for row_top, label, window, accent in (
        (top + 40, "Session", "current", theme["current"]),
        (top + 77, "Weekly", "weekly", theme["weekly"]),
    ):
        draw_split_row(
            draw, top=row_top, label=label, percent=usage.window(window).pct,
            reset_text=format_delta(usage.window(window).reset, usage.now), accent=accent, fonts=fonts,
            pace_info=window_pace(usage, window),
        )
    if usage.stale:
        box = (0, top, WIDTH, top + SPLIT_PANEL_H)
        image.paste(Image.blend(image.crop(box), Image.new("RGB", (WIDTH, SPLIT_PANEL_H), BG), 0.55), box)
        text = f"! stale since {updated}"
        width = ImageDraw.Draw(image).textlength(text, font=fonts["small"])
        ImageDraw.Draw(image).text((230 - width, top + 12), text, font=fonts["small"], fill=STATE_COLORS["warn"])


def render_split_frame(panels: list[Usage], animations: list[str] | None = None, index: int = 0) -> Image.Image:
    """Frame `index` of the split view. Each panel plays its own animation (default: just the idle bob); a shorter
    one simply loops again while a longer one is still going."""
    image = Image.new("RGB", (WIDTH, HEIGHT), BG)
    fonts = {
        "title": ImageFont.load_default(size=18), "pct": ImageFont.load_default(size=22),
        "small": ImageFont.load_default(size=11),
    }
    animations = animations or []
    for i, usage in enumerate(panels[:2]):
        spec = ANIMATIONS[animations[i] if i < len(animations) else "idle"]
        draw_split_panel(image, i * (SPLIT_PANEL_H + 4), usage, spec[index % len(spec)], fonts)
    ImageDraw.Draw(image).line((10, SPLIT_PANEL_H + 1, 230, SPLIT_PANEL_H + 1), fill=PILL_BG, width=2)  # divider
    return image


def render_split(panels: list[Usage], animations: list[str] | None = None) -> bytes:
    """Looping GIF with two providers' usage stacked on one screen, each mascot doing its own animation."""
    animations = animations or []
    length = max((len(ANIMATIONS[animations[i] if i < len(animations) else "idle"]) for i in range(len(panels[:2]))),
                 default=len(IDLE_BOB))
    return encode_gif([render_split_frame(panels, animations, index) for index in range(length)])
