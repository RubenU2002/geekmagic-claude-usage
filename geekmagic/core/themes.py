"""How each provider looks: its mascot, its colours. Pure data, shared by everything that draws something (the screens,
the tray icon)."""

from __future__ import annotations

BG = "#18160F"  # the screens' background: warm ink, laid out like a watch face (bold numbers, pill badges, no card boxes)
CURRENT_ACCENT = "#FA5407"  # Claude's vivid, saturated orange
WEEKLY_ACCENT = "#E8B24D"
ACCENT = CURRENT_ACCENT  # used by the mascot's body
INK = "#0F0D0B"  # the mascot's eyes and the dark parts of what it holds

# Pixel-art mascot bitmap: '1' = body, '2' = eye ink, '0' = background.
# 14 columns x 8 rows: squat and chubby: square torso, two stubby side arms, and four thin legs.
CLAUDE_BITMAP = (
    "00011111111000",
    "00011211211000",
    "01111111111110",
    "01111111111110",
    "00011111111000",
    "00011111111000",
    "00010100101000",
    "00010100101000",
)

# Codex's mascot: a little cloud (an original pixel-art take on a terminal-in-the-cloud), same 14x8 footprint as the
# Claude one. Its `>_` prompt is drawn per frame by the animations.
CODEX_BITMAP = (
    "00000000000000",
    "00000011110000",
    "00011011111100",
    "00111111111110",
    "01111111111111",
    "01111111111111",
    "01111111111111",
    "00111111111110",
)

# Per-provider look, picked by the provider's title: mascot bitmap + body colour, and the two bar accents.
THEMES = {
    "Claude": {"bitmap": CLAUDE_BITMAP, "body": ACCENT, "current": CURRENT_ACCENT, "weekly": WEEKLY_ACCENT},
    "Codex": {"bitmap": CODEX_BITMAP, "body": "#4F9DFF", "current": "#4F9DFF", "weekly": "#B79CFF"},
}


def hex_rgb(color: str) -> tuple[int, int, int]:
    return tuple(int(color[i:i + 2], 16) for i in (1, 3, 5))


def blend(color: str, over: str, amount: float) -> tuple[int, int, int]:
    """`color` mixed with `over` (0 = all `over`, 1 = all `color`)."""
    a, b = (tuple(int(c[i:i + 2], 16) for i in (1, 3, 5)) for c in (color, over))
    return tuple(round(b[k] + (a[k] - b[k]) * amount) for k in range(3))
