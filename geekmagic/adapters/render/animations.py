"""The mascots' animations, and which one plays when.

An animation is a tuple of `(mascot_dx, mascot_dy, extra)` per GIF frame, where `extra` is an optional
`(image, mascot_width) -> None` drawing hook (the "layers" below). Every animation lives side by side in ANIMATIONS, and
ANIMATION_GROUPS says which ones each provider rotates through.
"""

from __future__ import annotations

import random

from PIL import Image, ImageDraw

from geekmagic.core.model import Usage
from geekmagic.adapters.render.mascots import CODEX_BITMAP, INK, MASCOT_TOP
from geekmagic.adapters.render.props import LAPTOP_BEZEL, MUG_BITMAP, draw_bulb, draw_laptop, draw_mug, laptop_geometry


IDLE_BOB = (0, -1, -2, -1, 0, 1, 2, 1)  # a gentle idle bob, one loop


def combine(*layers):
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
        x, hidden_y, held_y = laptop_geometry(mascot_w)
        y = round(hidden_y - (hidden_y - held_y) * progress)
        draw_laptop(image, (x, y), 3)
    return draw


def _typing_layer(cursor_col: int):
    """A single blinking 'cursor' block sweeping across the held laptop's screen."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        x, _hidden_y, held_y = laptop_geometry(mascot_w)
        col = 1 + (cursor_col % 6)
        cx = x + col * 3
        cy = held_y + 1 * 3
        ImageDraw.Draw(image).rectangle((cx, cy, cx + 2, cy + 5), fill=LAPTOP_BEZEL)
    return draw


def _mug_layer(progress: float, steam_phase: int = 0):
    """The mug rising to `progress` (0..1) beside the mascot, with animated steam once held."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        if progress <= 0:
            return
        mug_cell = 2
        mug_w = mug_cell * len(MUG_BITMAP[0])
        x = 10 + mascot_w - mug_w + 2
        hidden_y, held_y = 34, 15
        y = round(hidden_y - (hidden_y - held_y) * progress)
        draw_mug(image, (x, y), mug_cell, steam_phase if progress >= 1 else 6)
    return draw


def _bulb_layer(progress: float, lit: bool):
    """The idea spark popping in beside the mascot's head, in the gap before the title."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        if progress <= 0:
            return
        bulb_cell = 2
        x = 10 + mascot_w - 8
        held_y, hidden_y = 2, 14
        y = round(hidden_y - (hidden_y - held_y) * progress)
        draw_bulb(image, (x, y), bulb_cell, lit)
    return draw


# Named animations, kept side by side so picking one never deletes another.
# Each entry is a tuple of (mascot dx, mascot dy, extra-drawing hook or None).
_IDLE_FRAMES = tuple((0, dy, None) for dy in IDLE_BOB)


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
        (0, 0, combine(_laptop_layer(1.0), _typing_layer(0))),
        (0, 0, combine(_laptop_layer(1.0), _typing_layer(1))),
        (0, 0, combine(_laptop_layer(1.0), _typing_layer(2))),
        (0, 0, combine(_laptop_layer(1.0), _typing_layer(3))),
        (0, 0, combine(_laptop_layer(1.0), _typing_layer(4))),
        (0, 0, combine(_laptop_layer(1.0), _typing_layer(5))),
        (0, 0, _laptop_layer(1.0)),
        (0, 0, _laptop_layer(0.5)),
    ),
}


DEFAULT_ANIMATION = "laptop"


# Each provider rotates through its own set; "auto"/"random" (the default) pick one, never the same twice in a row.
ANIMATION_GROUPS = {
    "Claude": ("idle", "laptop", "coffee", "eureka", "dance", "typing"),
    "Codex": ("prompt", "think", "sparkle", "bolt", "code", "hop"),
}


# --- Codex animations: all drawn on the 14x8 cloud grid (3 px cells), see CODEX_BITMAP -----------------

def cell(image: Image.Image, col: int, row: int, color: str = INK, dy: int = 0) -> None:
    """One 3x3 px cell of the mascot grid; `dy` follows a bobbing/hopping cloud."""
    x, y = 10 + col * 3, MASCOT_TOP + row * 3 + dy
    ImageDraw.Draw(image).rectangle((x, y, x + 2, y + 2), fill=color)


_CHEVRON = ((3, 4), (4, 5), (3, 6))  # the `>` of `>_`


_BOLT = ((8, 3), (7, 4), (8, 4), (6, 5), (7, 5), (7, 6), (8, 6), (7, 7), (6, 8))


_BRACKET_L = ((2, 4), (3, 4), (2, 5), (2, 6), (3, 6))


_BRACKET_R = ((11, 4), (10, 4), (11, 5), (11, 6), (10, 6))


_SPARK_COLOR = "#CFE3FF"


_SPARK_SPOTS = ((14, 3), (32, 2), (46, 5), (55, 15), (56, 27))  # top-left px of each sparkle, around the cloud


_FLASH_BODY = "#A9CFFF"


_BOLT_COLOR = "#FFD966"


_SHADOW_COLOR = "#3A352C"


def _prompt_layer(chevron: bool = True, dots: int = 0, cursor: int | None = None, dy: int = 0):
    """Codex's `>_` prompt on the cloud: a chevron, then `dots` typed dots, then a cursor block at column `cursor`."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        if chevron:
            for col, row in _CHEVRON:
                cell(image, col, row, dy=dy)
        for i in range(dots):
            cell(image, 7 + i * 2, 6, dy=dy)  # dots sit on the baseline, like periods
        if cursor is not None:
            for col in (cursor, cursor + 1):
                cell(image, col, 6, dy=dy)
    return draw


def _think_layer(dots: int, dy: int):
    """`dots` dots in the middle of a bobbing cloud: it's thinking."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        for i in range(dots):
            cell(image, 5 + 2 * i, 5, dy=dy)
    return draw


def _sparkle_layer(phase: int):
    """A resting `>_` with sparkles twinkling around the cloud (each one cycles off, dot, plus, dot)."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        for col, row in (*_CHEVRON, (7, 6), (8, 6)):
            cell(image, col, row)
        d = ImageDraw.Draw(image)
        for i, (x, y) in enumerate(_SPARK_SPOTS):
            stage = (phase + i) % 4
            if stage == 0:
                continue
            d.rectangle((x + 2, y + 2, x + 3, y + 3), fill=_SPARK_COLOR)  # centre dot
            if stage == 2:  # plus-shaped arms
                for ax, ay in ((x + 2, y), (x + 2, y + 4), (x, y + 2), (x + 4, y + 2)):
                    d.rectangle((ax, ay, ax + 1, ay + 1), fill=_SPARK_COLOR)
    return draw


def _bolt_layer(flash: bool):
    """Calm `>_`, or a lightning flash: the cloud lights up and a bolt strikes out of it."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        if not flash:
            for col, row in (*_CHEVRON, (7, 6), (8, 6)):
                cell(image, col, row)
            return
        for row, line in enumerate(CODEX_BITMAP):
            for col, ch in enumerate(line):
                if ch == "1":
                    cell(image, col, row, _FLASH_BODY)
        for col, row in _BOLT:
            cell(image, col, row, _BOLT_COLOR)
    return draw


def _code_layer(left: bool = False, right: bool = False, dots: int = 0):
    """`[ ... ]` brackets appearing on the cloud, with `dots` typed inside."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        for col, row in (_BRACKET_L if left else ()):
            cell(image, col, row)
        for col, row in (_BRACKET_R if right else ()):
            cell(image, col, row)
        for i in range(dots):
            cell(image, 4 + 2 * i, 5)
    return draw


def _hop_layer(dy: int, lift: int):
    """The cloud hopping: `>_` rides along, and the shadow on the ground shrinks as it rises (`lift` px)."""
    def draw(image: Image.Image, mascot_w: int) -> None:
        half = max(8, 17 - lift)
        ImageDraw.Draw(image).rectangle((31 - half, 36, 31 + half, 37), fill=_SHADOW_COLOR)
        for col, row in (*_CHEVRON, (7, 6), (8, 6)):
            cell(image, col, row, dy=dy)
    return draw


_HOP_DY = (0, 0, -3, -6, -8, -8, -6, -3, 0, 2, 0)


ANIMATIONS["prompt"] = (  # the prompt appears, the cursor blinks, then it types three dots
    (0, 0, None), (0, 0, None),
    (0, 0, _prompt_layer()),
    (0, 0, _prompt_layer(cursor=7)), (0, 0, _prompt_layer(cursor=7)),
    (0, 0, _prompt_layer()), (0, 0, _prompt_layer()),
    (0, 0, _prompt_layer(cursor=7)), (0, 0, _prompt_layer(cursor=7)),
    (0, 0, _prompt_layer(dots=1, cursor=9)), (0, 0, _prompt_layer(dots=1, cursor=9)),
    (0, 0, _prompt_layer(dots=2, cursor=11)), (0, 0, _prompt_layer(dots=2, cursor=11)),
    (0, 0, _prompt_layer(dots=3)), (0, 0, _prompt_layer(dots=3)), (0, 0, _prompt_layer(dots=3)),
    (0, 0, _prompt_layer()), (0, 0, _prompt_layer()),
)


ANIMATIONS["think"] = tuple(  # bobbing cloud (one bob cycle = smaller GIF), dots 0..3 cycling
    (0, IDLE_BOB[i], _think_layer((i // 2) % 4, IDLE_BOB[i])) for i in range(8)
)


ANIMATIONS["sparkle"] = tuple((0, 0, _sparkle_layer(i)) for i in range(8))


ANIMATIONS["bolt"] = (
    *((0, 0, _bolt_layer(False)),) * 5,
    (0, 0, _bolt_layer(True)), (0, 0, _bolt_layer(True)),
    (0, 0, _bolt_layer(False)),
    (0, 0, _bolt_layer(True)),
    *((0, 0, _bolt_layer(False)),) * 4,
)


ANIMATIONS["code"] = (  # [ ] brackets appear, three dots get typed inside
    (0, 0, None), (0, 0, None),
    (0, 0, _code_layer(left=True)), (0, 0, _code_layer(left=True)),
    (0, 0, _code_layer(left=True, right=True)), (0, 0, _code_layer(left=True, right=True)),
    (0, 0, _code_layer(True, True, 1)), (0, 0, _code_layer(True, True, 1)),
    (0, 0, _code_layer(True, True, 2)), (0, 0, _code_layer(True, True, 2)),
    (0, 0, _code_layer(True, True, 3)), (0, 0, _code_layer(True, True, 3)), (0, 0, _code_layer(True, True, 3)),
    (0, 0, _code_layer(True, True)), (0, 0, _code_layer(True, True)),
)


ANIMATIONS["hop"] = tuple((0, dy, _hop_layer(dy, -dy)) for dy in _HOP_DY)


RECENT_ANIMATIONS = 3  # an animation can't come back until this many different ones have played

WORKING_ANIMATION = {"Claude": "typing", "Codex": "code"}  # what each mascot does while its agent works
WAITING_ANIMATION = {"Claude": "eureka", "Codex": "sparkle"}  # ...and while it waits for you (an idea! / sparkles)


class Rotation:
    """Which animations each provider played lately, so a pick never repeats one that is still fresh in memory.

    The history only grows once an upload succeeds (see record), so a cancelled or failed upload can't make the next
    pick repeat something that is actually on screen.
    """

    def __init__(self) -> None:
        self.recent: dict[str, list[str]] = {}  # per provider title, oldest first

    def pick(self, title: str) -> str:
        """A random animation from the provider's own set, none of the last RECENT_ANIMATIONS shown."""
        group = ANIMATION_GROUPS.get(title, ANIMATION_GROUPS["Claude"])
        keep_out = min(RECENT_ANIMATIONS, len(group) - 1)  # always leave at least one candidate
        recent = self.recent.get(title, [])[-keep_out:] if keep_out else []
        return random.choice([a for a in group if a not in recent])

    def record(self, title: str, name: str) -> None:
        history = self.recent.setdefault(title, [])
        history.append(name)
        del history[:-RECENT_ANIMATIONS]

    def load(self, saved: dict) -> None:
        """Restore what was saved by dump(); names that no longer exist are dropped."""
        for title, names in saved.items():
            names = [names] if isinstance(names, str) else names  # older files stored just the last one
            self.recent[title] = [n for n in names if n in ANIMATIONS][-RECENT_ANIMATIONS:]

    def dump(self) -> dict[str, list[str]]:
        return {title: list(names) for title, names in self.recent.items()}

    def clear(self) -> None:
        self.recent.clear()

    # remembered across restarts (see geekmagic.app.persistence)
    def restore(self, saved: dict) -> None:
        animations = saved.get("animations")
        if isinstance(animations, dict):
            self.load(animations)

    def snapshot(self) -> dict:
        return {"animations": self.dump()}


def busy_animation(usage: Usage) -> str | None:
    """What the mascot does while its agent is busy (None when it isn't)."""
    title = usage.title
    if usage.waiting:
        return WAITING_ANIMATION.get(title, "idle")
    if usage.working:
        return WORKING_ANIMATION.get(title, "idle")
    return None


def animation_for(usage: Usage, requested: str, rotation: Rotation) -> str:
    """The animation a provider's screen should play: the one asked for if it's one of that provider's, else (for
    "auto" / "random") what its busy agent calls for, else a random one from its own set."""
    title = usage.title
    group = ANIMATION_GROUPS.get(title, ANIMATION_GROUPS["Claude"])
    if busy_animation(usage) and requested in ("auto", "random"):
        return busy_animation(usage)  # its agent is busy: the mascot works (or asks for you) too
    return requested if requested in group else rotation.pick(title)
