#!/usr/bin/env python3
"""Push Claude Code's /usage limits to a GeekMagic SmallTV Ultra (stock firmware).

Reads usage via Claude Code's own CLI (`claude /usage`), zero cost, and
renders a 240x240 animated GIF — a pixel-art mascot plus Current/Weekly
usage bars — uploaded straight into the device's stock Photo Album.
See ANIMATIONS for the available mascot animations.

Usage:
    python3 geekmagic_claude.py --ip 192.168.1.18
    python3 geekmagic_claude.py --ip 192.168.1.18 --loop 60
    python3 geekmagic_claude.py --ip 192.168.1.18 --animation coffee
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
import urllib.request
from datetime import datetime
from io import BytesIO
from zoneinfo import ZoneInfo

from PIL import Image, ImageDraw, ImageFont

WIDTH = HEIGHT = 240
IMAGE_NAME = "claude-usage.gif"
_OLD_IMAGE_NAMES = ("claude-usage.jpg",)  # cleaned up on first run after the GIF switch

# Claude's own palette (warm ink + a vivid, saturated orange), laid out
# like a watch face: bold numbers, pill badges, no card boxes.
# BG is deliberately not near-black: cheap small IPS panels show backlight
# bleed/unevenness much more on very dark fills, especially at the edges.
BG = "#2C2925"
PILL_BG = "#4A3F4D"
CURRENT_ACCENT = "#FA5407"
WEEKLY_ACCENT = "#E8B24D"
TEXT = "#F5F4EF"
MUTED = "#C7C2BC"
ACCENT = CURRENT_ACCENT  # used by the mascot's body

_SESSION_RE = re.compile(r"^Current session:\s*(\d+(?:\.\d+)?)% used(?:\s*·\s*resets\s+(.+))?$", re.MULTILINE)
_WEEK_RE = re.compile(r"^Current week(?:\s*\([^)]*\))?:\s*(\d+(?:\.\d+)?)% used(?:\s*·\s*resets\s+(.+))?$", re.MULTILINE)
_RESET_RE = re.compile(
    r"(?P<month>[A-Za-z]{3})\s+(?P<day>\d{1,2})\s+at\s+"
    r"(?P<hour>\d{1,2})(?::(?P<minute>\d{2}))?(?P<meridiem>am|pm)\s+\((?P<tz>[^)]+)\)",
    re.IGNORECASE,
)


class UsageError(RuntimeError):
    pass


def fetch_usage() -> dict:
    """Call Claude Code's built-in /usage command. Guaranteed $0 cost."""
    try:
        proc = subprocess.run(
            [
                "claude", "-p", "--safe-mode",
                "--output-format", "json",
                "--max-budget-usd", "0.000001",
                "--tools", "",
                "--no-session-persistence",
                "--no-chrome",
                "/usage",
            ],
            capture_output=True, text=True, timeout=30,
        )
    except FileNotFoundError as e:
        raise UsageError("`claude` CLI not found in PATH. Install/open Claude Code first.") from e
    except subprocess.TimeoutExpired as e:
        raise UsageError("Timed out waiting for `claude /usage`.") from e

    if proc.returncode != 0:
        raise UsageError(f"claude exited {proc.returncode}: {proc.stderr.strip()}")

    payload = json.loads(proc.stdout)

    cost = payload.get("total_cost_usd", 0)
    if float(cost) != 0:
        raise UsageError("Refusing result: /usage unexpectedly incurred cost.")

    result = payload.get("result")
    if payload.get("is_error") or not isinstance(result, str):
        raise UsageError("Claude Code did not return usage text.")
    return _parse(result)


def _parse(text: str) -> dict:
    now = datetime.now().astimezone()
    session = _SESSION_RE.search(text)
    week = _WEEK_RE.search(text)
    if not session and not week:
        raise UsageError("Could not find usage lines in /usage output:\n" + text)
    return {
        "current_pct": float(session.group(1)) if session else None,
        "current_reset": _parse_reset(session.group(2), now) if session and session.group(2) else None,
        "weekly_pct": float(week.group(1)) if week else None,
        "weekly_reset": _parse_reset(week.group(2), now) if week and week.group(2) else None,
        "now": now,
    }


def _parse_reset(text: str, now: datetime) -> datetime | None:
    m = _RESET_RE.search(text)
    if not m:
        return None
    tz = ZoneInfo(m.group("tz"))
    month = datetime.strptime(m.group("month"), "%b").month
    hour = int(m.group("hour")) % 12
    if m.group("meridiem").lower() == "pm":
        hour += 12
    minute = int(m.group("minute") or 0)
    local_now = now.astimezone(tz)
    candidate = None
    for year in (local_now.year, local_now.year + 1):
        candidate = datetime(year, month, int(m.group("day")), hour, minute, tzinfo=tz)
        if candidate >= local_now:
            return candidate
    return candidate


def _format_delta(reset_at: datetime | None, now: datetime) -> str:
    if reset_at is None:
        return "Resets in --"
    delta = reset_at - now
    total_minutes = max(0, int(delta.total_seconds() // 60))
    days, rem_minutes = divmod(total_minutes, 24 * 60)
    hours, minutes = divmod(rem_minutes, 60)
    if days >= 1:
        return f"Resets in {days}d"
    return f"Resets in {hours}h {minutes}m"


# Pixel-art mascot bitmap: '1' = orange body, '2' = black eye, '0' = background.
# 14 columns x 8 rows — squat and chubby: square torso, two stubby side
# arms, and four thin legs, without the extra height of a taller torso.
_MASCOT_BITMAP = (
    "00011111111000",
    "00011211211000",
    "01111111111110",
    "01111111111110",
    "00011111111000",
    "00011111111000",
    "00010100101000",
    "00010100101000",
)


def _draw_mascot(image: Image.Image, top_left: tuple[int, int], cell_size: int) -> None:
    """Blocky pixel-art mascot, drawn cell by cell so it stays crisp at small sizes."""
    x0, y0 = top_left
    draw = ImageDraw.Draw(image)
    for row, line in enumerate(_MASCOT_BITMAP):
        for col, cell in enumerate(line):
            if cell == "0":
                continue
            fill = ACCENT if cell == "1" else "#0F0D0B"
            x = x0 + col * cell_size
            y = y0 + row * cell_size
            draw.rectangle((x, y, x + cell_size - 1, y + cell_size - 1), fill=fill)


# A tiny pixel-art laptop the mascot pulls out mid-loop: 'B' = bezel/base,
# 'G' = the glowing screen, '.' = background.
_LAPTOP_BITMAP = (
    "BBBBBBBB",
    "BGGGGGGB",
    "BGGGGGGB",
    "BBBBBBBB",
    ".BBBBBB.",
    "BBBBBBBB",
)
_LAPTOP_BEZEL = "#4A3F4D"
_LAPTOP_GLOW = "#FFE9C7"


def _draw_laptop(image: Image.Image, top_left: tuple[int, int], cell_size: int) -> None:
    x0, y0 = top_left
    draw = ImageDraw.Draw(image)
    for row, line in enumerate(_LAPTOP_BITMAP):
        for col, cell in enumerate(line):
            if cell == ".":
                continue
            fill = _LAPTOP_BEZEL if cell == "B" else _LAPTOP_GLOW
            x = x0 + col * cell_size
            y = y0 + row * cell_size
            draw.rectangle((x, y, x + cell_size - 1, y + cell_size - 1), fill=fill)


def _laptop_geometry(mascot_w: int, cell_size: int = 3) -> tuple[int, int, int]:
    """(x, hidden_y, held_y) shared by the laptop and the typing-cursor overlay."""
    laptop_w = cell_size * len(_LAPTOP_BITMAP[0])
    x = 10 + (mascot_w - laptop_w) // 2
    return x, 34, 21


# A tiny pixel-art mug the mascot pulls out mid-loop: 'B' = ceramic, 'C' = coffee.
_MUG_BITMAP = (
    "BBBBBB",
    "BCCCCB",
    "BCCCCB",
    "BCCCCB",
    "BBBBBB",
)
_MUG_CERAMIC = "#F5F4EF"
_MUG_COFFEE = "#5A3A22"
_STEAM_COLOR = "#C7C2BC"


def _draw_mug(image: Image.Image, top_left: tuple[int, int], cell_size: int, steam_phase: int) -> None:
    x0, y0 = top_left
    draw = ImageDraw.Draw(image)
    for row, line in enumerate(_MUG_BITMAP):
        for col, cell in enumerate(line):
            fill = _MUG_CERAMIC if cell == "B" else _MUG_COFFEE
            x = x0 + col * cell_size
            y = y0 + row * cell_size
            draw.rectangle((x, y, x + cell_size - 1, y + cell_size - 1), fill=fill)

    # Two wisps of steam, each drifting up and fading over a 3-step cycle.
    for wisp, col in enumerate((1, 4)):
        phase = (steam_phase + wisp * 2) % 6
        if phase >= 4:
            continue
        x = x0 + col * cell_size
        y = y0 - (phase + 1) * cell_size
        draw.rectangle((x, y, x + cell_size - 1, y + cell_size - 1), fill=_STEAM_COLOR)


# A tiny "idea" spark for the "eureka" moment — a diamond burst, simplest
# shape that still reads clearly at a few pixels across.
_BULB_BITMAP = (
    "..G..",
    ".GGG.",
    "GGGGG",
    ".GGG.",
    "..G..",
)
_BULB_GLOW_ON = "#FFD966"
_BULB_GLOW_OFF = "#6B5E3E"


def _draw_bulb(image: Image.Image, top_left: tuple[int, int], cell_size: int, lit: bool) -> None:
    x0, y0 = top_left
    draw = ImageDraw.Draw(image)
    glow = _BULB_GLOW_ON if lit else _BULB_GLOW_OFF
    for row, line in enumerate(_BULB_BITMAP):
        for col, cell in enumerate(line):
            if cell == ".":
                continue
            fill = glow
            x = x0 + col * cell_size
            y = y0 + row * cell_size
            draw.rectangle((x, y, x + cell_size - 1, y + cell_size - 1), fill=fill)


_MASCOT_TOP = 10
_IDLE_BOB = (0, -1, -2, -1, 0, 1, 2, 1)  # a gentle idle bob, one loop


def _combine(*layers):
    """Chain several per-frame drawing hooks into one."""
    active = [layer for layer in layers if layer is not None]

    def draw(image: Image.Image, mascot_w: int) -> None:
        for layer in active:
            layer(image, mascot_w)
    return draw


def _laptop_layer(progress: float):
    """The laptop rising to `progress` (0..1) in front of the mascot."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        if progress <= 0:
            return
        x, hidden_y, held_y = _laptop_geometry(mascot_w)
        y = round(hidden_y - (hidden_y - held_y) * progress)
        _draw_laptop(image, (x, y), 3)
    return draw


def _typing_layer(cursor_col: int):
    """A single blinking 'cursor' block sweeping across the held laptop's screen."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        x, _hidden_y, held_y = _laptop_geometry(mascot_w)
        col = 1 + (cursor_col % 6)
        cx = x + col * 3
        cy = held_y + 1 * 3
        ImageDraw.Draw(image).rectangle((cx, cy, cx + 2, cy + 5), fill=_LAPTOP_BEZEL)
    return draw


def _mug_layer(progress: float, steam_phase: int = 0):
    """The mug rising to `progress` (0..1) beside the mascot, with animated steam once held."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        if progress <= 0:
            return
        mug_cell = 2
        mug_w = mug_cell * len(_MUG_BITMAP[0])
        x = 10 + mascot_w - mug_w + 2
        hidden_y, held_y = 34, 15
        y = round(hidden_y - (hidden_y - held_y) * progress)
        _draw_mug(image, (x, y), mug_cell, steam_phase if progress >= 1 else 6)
    return draw


def _bulb_layer(progress: float, lit: bool):
    """The idea spark popping in beside the mascot's head, in the gap before the title."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        if progress <= 0:
            return
        bulb_cell = 2
        bulb_w = bulb_cell * len(_BULB_BITMAP[0])
        x = 10 + mascot_w - 8
        held_y, hidden_y = 2, 14
        y = round(hidden_y - (hidden_y - held_y) * progress)
        _draw_bulb(image, (x, y), bulb_cell, lit)
    return draw


# Named animations, kept side by side so picking one never deletes another.
# Each entry is a tuple of (mascot dx, mascot dy, extra-drawing hook or None).
_IDLE_FRAMES = tuple((0, dy, None) for dy in _IDLE_BOB)

ANIMATIONS: dict[str, tuple[tuple[int, int, object], ...]] = {
    "idle": _IDLE_FRAMES,
    "laptop": (
        *_IDLE_FRAMES,
        (0, 0, _laptop_layer(0.35)), (0, 0, _laptop_layer(0.7)), (0, 0, _laptop_layer(1.0)),
        (0, 1, _laptop_layer(1.0)), (0, 2, _laptop_layer(1.0)), (0, 1, _laptop_layer(1.0)), (0, 0, _laptop_layer(1.0)),
        (0, 0, _laptop_layer(0.5)),
    ),
    "coffee": (
        *_IDLE_FRAMES,
        (0, 0, _mug_layer(0.4)), (0, 0, _mug_layer(0.7)), (0, 0, _mug_layer(1.0, 0)),
        (0, 0, _mug_layer(1.0, 1)), (0, 0, _mug_layer(1.0, 2)), (0, 0, _mug_layer(1.0, 3)),
        (0, 0, _mug_layer(1.0, 4)), (0, 0, _mug_layer(1.0, 5)), (0, 0, _mug_layer(1.0, 0)),
        (0, 0, _mug_layer(0.5)),
    ),
    "eureka": (
        *_IDLE_FRAMES,
        (0, 0, _bulb_layer(0.5, True)), (0, 0, _bulb_layer(1.0, True)),
        (0, 0, _bulb_layer(1.0, False)), (0, 0, _bulb_layer(1.0, True)),
        (0, 0, _bulb_layer(1.0, False)), (0, 0, _bulb_layer(1.0, True)),
        (0, 0, _bulb_layer(0.5, True)),
    ),
    "dance": (
        (0, 0, None), (2, -2, None), (3, 0, None), (2, 2, None),
        (0, 0, None), (-2, -2, None), (-3, 0, None), (-2, 2, None),
    ),
    "typing": (
        *_IDLE_FRAMES,
        (0, 0, _laptop_layer(0.35)), (0, 0, _laptop_layer(0.7)),
        (0, 0, _combine(_laptop_layer(1.0), _typing_layer(0))),
        (0, 0, _combine(_laptop_layer(1.0), _typing_layer(1))),
        (0, 0, _combine(_laptop_layer(1.0), _typing_layer(2))),
        (0, 0, _combine(_laptop_layer(1.0), _typing_layer(3))),
        (0, 0, _combine(_laptop_layer(1.0), _typing_layer(4))),
        (0, 0, _combine(_laptop_layer(1.0), _typing_layer(5))),
        (0, 0, _laptop_layer(1.0)),
        (0, 0, _laptop_layer(0.5)),
    ),
}
DEFAULT_ANIMATION = "laptop"


def _render_frame(usage: dict, mascot_dx: int = 0, mascot_dy: int = 0, extra=None) -> Image.Image:
    image = Image.new("RGB", (WIDTH, HEIGHT), BG)
    draw = ImageDraw.Draw(image)

    title_font = ImageFont.load_default(size=26)
    pill_font = ImageFont.load_default(size=14)
    pct_font = ImageFont.load_default(size=30)
    reset_font = ImageFont.load_default(size=14)
    footer_font = ImageFont.load_default(size=13)

    mascot_cell = 3
    mascot_w = mascot_cell * len(_MASCOT_BITMAP[0])
    _draw_mascot(image, (10 + mascot_dx, _MASCOT_TOP + mascot_dy), mascot_cell)

    if extra is not None:
        extra(image, mascot_w)

    draw.text((10 + mascot_w + 10, 8), "Usage", font=title_font, fill=TEXT)

    _draw_section(
        draw, top=40, label="Current", percent=usage["current_pct"],
        reset_text=_format_delta(usage["current_reset"], usage["now"]),
        accent=CURRENT_ACCENT,
        pill_font=pill_font, pct_font=pct_font, reset_font=reset_font,
    )
    _draw_section(
        draw, top=134, label="Weekly", percent=usage["weekly_pct"],
        reset_text=_format_delta(usage["weekly_reset"], usage["now"]),
        accent=WEEKLY_ACCENT,
        pill_font=pill_font, pct_font=pct_font, reset_font=reset_font,
    )

    footer_text = f"* Updated {usage['now']:%H:%M}"
    bbox = draw.textbbox((0, 0), footer_text, font=footer_font)
    draw.text((((240 - (bbox[2] - bbox[0])) // 2), 223), footer_text, font=footer_font, fill=CURRENT_ACCENT)

    return image


def render(usage: dict) -> bytes:
    """Static JPEG (single frame, mascot at rest)."""
    buf = BytesIO()
    _render_frame(usage).save(buf, format="JPEG", quality=92)
    return buf.getvalue()


def render_animation(usage: dict, animation: str = DEFAULT_ANIMATION) -> bytes:
    """Looping GIF for one of the named ANIMATIONS presets."""
    frames_spec = ANIMATIONS[animation]
    frames = [_render_frame(usage, dx, dy, extra) for dx, dy, extra in frames_spec]
    base = frames[0].convert("P", palette=Image.ADAPTIVE, colors=64)
    quantized = [f.quantize(palette=base) for f in frames]
    buf = BytesIO()
    quantized[0].save(
        buf, format="GIF", save_all=True, append_images=quantized[1:],
        duration=130, loop=0, disposal=2, optimize=False,
    )
    return buf.getvalue()


def _draw_section(draw, *, top, label, percent, reset_text, accent, pill_font, pct_font, reset_font) -> None:
    panel_left, panel_right = 10, 230
    pad = 4
    pct_text = f"{percent:.0f}%" if percent is not None else "--%"
    draw.text((panel_left + pad, top + 6), pct_text, font=pct_font, fill=TEXT)

    pill_bbox = draw.textbbox((0, 0), label, font=pill_font)
    pill_text_w = pill_bbox[2] - pill_bbox[0]
    pill_h = 26
    pill_w = pill_text_w + 28
    pill_left = panel_right - pad - pill_w
    pill_top = top + 10
    draw.rounded_rectangle((pill_left, pill_top, pill_left + pill_w, pill_top + pill_h), radius=pill_h // 2, fill=PILL_BG)
    draw.text((pill_left + (pill_w - pill_text_w) // 2, pill_top + (pill_h - pill_bbox[3]) // 2), label, font=pill_font, fill=TEXT)

    bar_left, bar_top, bar_right, bar_h = panel_left + pad, top + 44, panel_right - pad, 18
    draw.rounded_rectangle((bar_left, bar_top, bar_right, bar_top + bar_h), radius=9, fill=PILL_BG)
    if percent:
        fill_w = round((bar_right - bar_left) * min(percent, 100) / 100)
        if fill_w > 0:
            draw.rounded_rectangle(
                (bar_left, bar_top, bar_left + fill_w, bar_top + bar_h),
                radius=min(9, max(3, fill_w // 2)), fill=accent,
            )

    draw.text((panel_left + pad, top + 68), reset_text, font=reset_font, fill=MUTED)


def upload(ip: str, image_bytes: bytes, filename: str = IMAGE_NAME, content_type: str = "image/gif") -> None:
    boundary = "----geekmagicclaude"
    body = (
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="file"; filename="{filename}"\r\n'
        f"Content-Type: {content_type}\r\n\r\n"
    ).encode() + image_bytes + f"\r\n--{boundary}--\r\n".encode()

    req = urllib.request.Request(
        f"http://{ip}/doUpload?dir=/image/",
        data=body,
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=10) as resp:
        if resp.status >= 300:
            raise UsageError(f"Upload failed: HTTP {resp.status}")

    urllib.request.urlopen(f"http://{ip}/set?theme=3", timeout=10).read()
    urllib.request.urlopen(f"http://{ip}/set?img=/image/{filename}", timeout=10).read()


def _cleanup_old_images(ip: str) -> None:
    for name in _OLD_IMAGE_NAMES:
        try:
            urllib.request.urlopen(f"http://{ip}/delete?file=/image/{name}", timeout=10).read()
        except OSError:
            pass


def run_once(ip: str, animation: str) -> None:
    usage = fetch_usage()
    if animation == "random":
        import random
        animation = random.choice(list(ANIMATIONS))
    gif_bytes = render_animation(usage, animation)
    upload(ip, gif_bytes, IMAGE_NAME, "image/gif")
    _cleanup_old_images(ip)
    print(
        f"[{usage['now']:%H:%M:%S}] pushed [{animation}] ({len(gif_bytes) / 1024:.1f} KB) — "
        f"current {usage['current_pct']}% / weekly {usage['weekly_pct']}%"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ip", required=True, help="GeekMagic device IP, e.g. 192.168.1.18")
    parser.add_argument("--loop", type=int, metavar="SECONDS", help="repeat forever every N seconds")
    parser.add_argument(
        "--animation", default=DEFAULT_ANIMATION, choices=[*ANIMATIONS, "random"],
        help="which named animation to show (or 'random' to pick a different one each push)",
    )
    args = parser.parse_args()

    try:
        if args.loop:
            while True:
                try:
                    run_once(args.ip, args.animation)
                except UsageError as e:
                    print(f"warning: {e}", file=sys.stderr)
                time.sleep(args.loop)
        else:
            run_once(args.ip, args.animation)
    except UsageError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
