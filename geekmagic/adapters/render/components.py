"""The pieces the screens are drawn from: bars, pills, the pace line, the "working" tag, share rows, the week chart."""

from __future__ import annotations

from datetime import datetime

from PIL import Image, ImageDraw

from geekmagic.core.model import Usage
from geekmagic.core import alerts, pace
from geekmagic.core import activity as activity_stats
from geekmagic.adapters.render.mascots import THEMES, draw_mascot, theme_for
from geekmagic.adapters.render.palette import BG, MUTED, PILL_BG, SPLIT_PANEL_H, STATE_COLORS, TEXT, WAITING_COLOR, WIDTH, WORKING_COLOR, blend


def clock(moment: datetime) -> str:
    """12-hour clock time, "2:57 PM". Built by hand: strftime's %p follows the system locale ("p. m." in Spanish)."""
    return f"{moment.hour % 12 or 12}:{moment.minute:02d} {'AM' if moment.hour < 12 else 'PM'}"


def format_delta(reset_at: datetime | None, now: datetime) -> str:
    if reset_at is None:
        return "Resets in --"
    delta = reset_at - now
    total_minutes = max(0, int(delta.total_seconds() // 60))
    days, rem_minutes = divmod(total_minutes, 24 * 60)
    hours, minutes = divmod(rem_minutes, 60)
    if days >= 1:
        return f"Resets in {days}d"
    return f"Resets in {hours}h {minutes}m"


def resets_chip(usage: Usage) -> tuple[str, str] | None:
    """(text, colour) for the free-resets note in the corner of Codex's screen: "3 resets · 2d" (days until the
    next one expires; yellow from 3 days, red from 1 so it gets used in time)."""
    extra = usage.codex_extra
    count = extra.free_resets if extra else 0
    if usage.title != "Codex" or not count:
        return None
    days = extra.next_reset_expires_days
    text = f"{count} reset{'' if count == 1 else 's'}" + (f" · {days}d" if days is not None else "")
    color = STATE_COLORS["crit"] if days is not None and days <= 1 else STATE_COLORS["warn"] if days is not None and days <= 3 else THEMES["Codex"]["weekly"]
    return text, color


# --- Stats view: activity numbers for both providers ------------------------------------------------------

def fit(draw, text: str, font, width: float) -> str:
    """`text` shortened with "..." to fit `width` pixels."""
    if draw.textlength(text, font=font) <= width:
        return text
    while text and draw.textlength(text + "...", font=font) > width:
        text = text[:-1]
    return text.rstrip() + "..."


def draw_week_chart(draw, days: list[dict], *, left: int, right: int, baseline: int, height: int, color: str, fonts: dict) -> None:
    """Requests per day for the last week: one bar per day (today's in full colour, earlier days dimmer), the number
    above each bar and the weekday under it. Scaled to the provider's own busiest day, so the shapes compare."""
    slot = (right - left) / len(days)
    top_value = max((d["requests"] for d in days), default=0)
    dim = blend(color, BG, 0.45)
    for i, day in enumerate(days):
        n = day["requests"]
        x = left + i * slot
        bar_w = min(20, slot - 6)
        x0 = round(x + (slot - bar_w) / 2)
        h = max(2, round(height * n / top_value)) if top_value and n else 2
        today = i == len(days) - 1
        draw.rectangle((x0, baseline - h, x0 + round(bar_w) - 1, baseline - 1), fill=color if today and n else (dim if n else PILL_BG))
        label = f"{n:,}" if n < 10000 else f"{n // 1000}k"
        w = draw.textlength(label, font=fonts["tiny"])
        draw.text((x0 + bar_w / 2 - w / 2, baseline - h - 11), label, font=fonts["tiny"], fill=TEXT if today else MUTED)
        name = activity_stats.weekday(day["date"])
        w = draw.textlength(name, font=fonts["tiny"])
        draw.text((x0 + bar_w / 2 - w / 2, baseline + 1), name, font=fonts["tiny"], fill=TEXT if today else MUTED)


def busy_style(usage: Usage) -> tuple[str, str] | None:
    """(label, colour) of the badge for an agent that is waiting for you or working, else None. Waiting wins."""
    if usage.waiting:
        return "Waiting", WAITING_COLOR
    if usage.working:
        return "Working", WORKING_COLOR
    return None


def draw_working_dot(draw, x: float, y: float, color: str = WORKING_COLOR) -> None:
    """A small dot with a halo: "this agent is busy" (green) or "is waiting for you" (amber), in a panel header."""
    draw.ellipse((x - 5, y - 5, x + 5, y + 5), fill=blend(color, BG, 0.3))
    draw.ellipse((x - 3, y - 3, x + 3, y + 3), fill=color)


def draw_busy_tag(draw, y: float, usage: Usage, font, left_of: float | None = None) -> None:
    """In a panel header, the dot and its word ("Working" / "Waiting"). A dot alone is easy to miss on a small screen;
    the word makes it unmistakable. By default it's right-aligned on the line under the clock (the split view's
    header is crowded: beside the clock it ran into the provider's name); with `left_of` it sits just left of that x
    instead, for headers with room. Nothing if the agent isn't busy."""
    busy = busy_style(usage)
    if not busy:
        return
    label, color = busy
    width = draw.textlength(label, font=font)
    if left_of is None:
        draw_working_dot(draw, 230 - width - 11, y + 7, color)
        draw.text((230 - width, y), label, font=font, fill=color)
    else:
        draw_working_dot(draw, left_of - 8, y + 7, color)
        draw.text((left_of - 16 - width, y), label, font=font, fill=color)


def draw_panel_header(image: Image.Image, top: int, usage: Usage, fonts: dict) -> None:
    """The header every two-provider view shares: small mascot, name, time (or the working dot beside it)."""
    theme = theme_for(usage)
    draw = ImageDraw.Draw(image)
    draw_mascot(image, (10, top + 8), 2, theme)
    draw.text((46, top + 5), usage.title, font=fonts["title"], fill=TEXT)
    if not usage.stale:
        updated = clock(usage.now)
        width = draw.textlength(updated, font=fonts["small"])
        draw.text((230 - width, top + 10), updated, font=fonts["small"], fill=theme["current"])
        draw_busy_tag(draw, top + 10, usage, fonts["small"], left_of=230 - width)


def dim_stale_panel(image: Image.Image, top: int, usage: Usage, fonts: dict) -> None:
    """A panel whose numbers couldn't be refreshed: everything dimmed, and since when."""
    if not usage.stale:
        return
    box = (0, top, WIDTH, top + SPLIT_PANEL_H)
    image.paste(Image.blend(image.crop(box), Image.new("RGB", (WIDTH, SPLIT_PANEL_H), BG), 0.55), box)
    text = f"! stale since {clock(usage.now)}"
    draw = ImageDraw.Draw(image)
    draw.text((230 - draw.textlength(text, font=fonts["small"]), top + 10), text, font=fonts["small"], fill=STATE_COLORS["warn"])


def draw_share_rows(draw, items: list[dict], total: int, *, left: int, width: int, top: int, color: str, fonts: dict) -> None:
    """Up to three rows: a name, its share in %, and a bar under them."""
    for i, (name, share) in enumerate(activity_stats.shares(items, total, 3)):
        y = top + i * 25
        pct = f"{share * 100:.0f}%" if share >= 0.01 else "<1%"
        pct_w = draw.textlength(pct, font=fonts["tiny"])
        draw.text((left, y), fit(draw, name, fonts["tiny"], width - pct_w - 6), font=fonts["tiny"], fill=TEXT)
        draw.text((left + width - pct_w, y), pct, font=fonts["tiny"], fill=MUTED)
        draw.rounded_rectangle((left, y + 14, left + width, y + 19), radius=2, fill=PILL_BG)
        draw.rounded_rectangle((left, y + 14, left + max(3, round(width * share)), y + 19), radius=2, fill=color)


def draw_section(
    draw, *, top, label, percent, reset_text, accent, pill_font, pct_font, reset_font, pace_info=None, pace_font=None,
) -> None:
    panel_left, panel_right = 10, 230
    pad = 4
    pct_text = f"{percent:.0f}%" if percent is not None else "--%"
    state = alerts.bar_state(percent)  # past the warn/crit thresholds the number and bar change colour
    accent = STATE_COLORS.get(state, accent)
    draw.text((panel_left + pad, top + 6), pct_text, font=pct_font, fill=STATE_COLORS.get(state, TEXT))

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
    draw_pace(draw, pace_info, right=panel_right - pad, y=top + 70, left_end=panel_left + pad + draw.textlength(reset_text, font=reset_font), font=pace_font)


def draw_pace(draw, pace_info, *, right: int, y: int, left_end: float, font) -> None:
    """The projection ("Full in 1h 30m" / "~62% at reset"), right-aligned; skipped if it wouldn't fit next to the
    reset countdown, which matters more."""
    if not pace_info:
        return
    text, state = pace_info
    width = draw.textlength(text, font=font)
    if left_end + 10 < right - width:
        draw.text((right - width, y), text, font=font, fill=STATE_COLORS.get(state, MUTED))


def window_pace(usage: Usage, window: str):
    """(text, state) projection for a usage window, or None (also None while the numbers are stale)."""
    if usage.stale:
        return None
    return pace.describe(usage.pace.get(window))


def draw_split_row(draw, *, top, label, percent, reset_text, accent, fonts, pace_info=None) -> None:
    """One compact usage row: big number + bar on the first line, label + reset countdown under it."""
    state = alerts.bar_state(percent)
    color = STATE_COLORS.get(state, accent)
    pct_text = f"{percent:.0f}%" if percent is not None else "--%"
    draw.text((10, top - 1), pct_text, font=fonts["pct"], fill=STATE_COLORS.get(state, TEXT))
    bar_left, bar_right, bar_top, bar_h = 68, 230, top + 4, 12
    draw.rounded_rectangle((bar_left, bar_top, bar_right, bar_top + bar_h), radius=6, fill=PILL_BG)
    if percent:
        fill_w = round((bar_right - bar_left) * min(percent, 100) / 100)
        if fill_w > 0:
            draw.rounded_rectangle(
                (bar_left, bar_top, bar_left + fill_w, bar_top + bar_h), radius=min(6, max(2, fill_w // 2)), fill=color,
            )
    draw.text((10, top + 26), label, font=fonts["small"], fill=MUTED)
    draw.text((bar_left, top + 26), reset_text, font=fonts["small"], fill=MUTED)
    draw_pace(draw, pace_info, right=230, y=top + 26, left_end=bar_left + draw.textlength(reset_text, font=fonts["small"]), font=fonts["small"])
