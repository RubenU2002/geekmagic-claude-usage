"""The views of two providers' local activity: stats, projects and models, peak hours."""

from __future__ import annotations

from PIL import Image, ImageDraw, ImageFont

from geekmagic.core.model import Usage
from geekmagic.core import activity as activity_stats
from geekmagic.adapters.render.components import dim_stale_panel, draw_panel_header, draw_share_rows, draw_week_chart
from geekmagic.adapters.render.gif import encode_gif
from geekmagic.adapters.render.mascots import theme_for
from geekmagic.adapters.render.palette import BG, HEIGHT, MUTED, PILL_BG, SPLIT_PANEL_H, TEXT, WIDTH, blend


def draw_stats_panel(image: Image.Image, top: int, usage: Usage, fonts: dict) -> None:
    """One provider's activity: requests and sessions for 24 h and 7 days, and requests per day over the last week.
    The very same layout and numbers for every provider, so they can be compared at a glance."""
    theme = theme_for(usage)
    draw = ImageDraw.Draw(image)
    draw_panel_header(image, top, usage, fonts)
    activity = usage.activity
    if not activity:
        draw.text((10, top + 40), "No local activity logs found" if activity == {} else "Counting...",
                  font=fonts["body"], fill=MUTED)
    else:
        for i, label in enumerate(("24h", "7d")):
            draw.text((10, top + 28 + i * 15), activity_stats.counts_text(label, activity[label]), font=fonts["body"], fill=TEXT)
        draw_week_chart(draw, activity["days"], left=10, right=230, baseline=top + 106, height=34,
                         color=theme["body"], fonts=fonts)
    dim_stale_panel(image, top, usage, fonts)


def draw_breakdown_panel(image: Image.Image, top: int, usage: Usage, fonts: dict) -> None:
    """Where this week's requests went: the top projects on the left, the top models on the right (shares of the
    provider's own total, same layout for every provider)."""
    theme = theme_for(usage)
    draw = ImageDraw.Draw(image)
    draw_panel_header(image, top, usage, fonts)
    activity = usage.activity
    if not activity or not activity.get("total"):
        draw.text((10, top + 40), "No local activity logs found" if activity == {} else
                  "Counting..." if activity is None else "No activity in the last 7 days", font=fonts["body"], fill=MUTED)
    else:
        for left, label, key, color in ((10, "Projects · 7d", "projects", theme["body"]), (126, "Models · 7d", "models", theme["weekly"])):
            draw.text((left, top + 28), label, font=fonts["tiny"], fill=MUTED)
            draw_share_rows(draw, activity[key], activity["total"], left=left, width=104, top=top + 42, color=color, fonts=fonts)
    dim_stale_panel(image, top, usage, fonts)


def draw_hours_panel(image: Image.Image, top: int, usage: Usage, fonts: dict) -> None:
    """When you work: requests for each hour of the day over the last four weeks, with the busiest stretch called out."""
    theme = theme_for(usage)
    draw = ImageDraw.Draw(image)
    draw_panel_header(image, top, usage, fonts)
    activity = usage.activity
    hours = (activity or {}).get("hours")
    if not hours or not any(hours):
        draw.text((10, top + 40), "No local activity logs found" if activity == {} else
                  "Counting..." if activity is None else "No activity in the last 4 weeks", font=fonts["body"], fill=MUTED)
    else:
        window = activity_stats.busiest_hours(hours)
        draw.text((10, top + 27), f"Busiest {activity_stats.hours_text(window)} · last {activity_stats.HOURS_DAYS} days",
                  font=fonts["body"], fill=TEXT)
        peak, baseline, height = max(hours), top + 102, 46
        dim = blend(theme["body"], BG, 0.45)
        for hour, n in enumerate(hours):
            x = 12 + hour * 9
            h = max(2, round(height * n / peak)) if n else 2
            inside = window[0] <= hour < window[0] + 2 or (window[1] < window[0] and hour < window[1])  # (wraps midnight)
            draw.rectangle((x, baseline - h, x + 6, baseline - 1), fill=theme["body"] if inside and n else (dim if n else PILL_BG))
        for hour, label in ((0, "12a"), (6, "6a"), (12, "12p"), (18, "6p")):
            draw.text((12 + hour * 9, baseline + 1), label, font=fonts["tiny"], fill=MUTED)
    dim_stale_panel(image, top, usage, fonts)


PANEL_DRAWERS = {"stats": draw_stats_panel, "breakdown": draw_breakdown_panel, "hours": draw_hours_panel}


def render_panel_view(panels: list[Usage], drawer) -> Image.Image:
    image = Image.new("RGB", (WIDTH, HEIGHT), BG)
    fonts = {"title": ImageFont.load_default(size=17), "body": ImageFont.load_default(size=12),
             "small": ImageFont.load_default(size=11), "tiny": ImageFont.load_default(size=10)}
    for i, usage in enumerate(panels[:2]):
        drawer(image, i * (SPLIT_PANEL_H + 4), usage, fonts)
    ImageDraw.Draw(image).line((10, SPLIT_PANEL_H + 1, 230, SPLIT_PANEL_H + 1), fill=PILL_BG, width=2)
    return image


def render_stats_frame(panels: list[Usage]) -> Image.Image:
    return render_panel_view(panels, draw_stats_panel)


def render_breakdown_frame(panels: list[Usage]) -> Image.Image:
    return render_panel_view(panels, draw_breakdown_panel)


def render_hours_frame(panels: list[Usage]) -> Image.Image:
    return render_panel_view(panels, draw_hours_panel)


def render_stats(panels: list[Usage]) -> bytes:
    """A still GIF (so a small upload) with both providers' activity stats."""
    return encode_gif([render_stats_frame(panels)])


def render_breakdown(panels: list[Usage]) -> bytes:
    return encode_gif([render_breakdown_frame(panels)])


def render_hours(panels: list[Usage]) -> bytes:
    return encode_gif([render_hours_frame(panels)])
