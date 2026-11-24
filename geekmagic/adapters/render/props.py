"""The little things a mascot holds: a laptop, a mug of coffee, an idea spark."""

from __future__ import annotations

from PIL import Image, ImageDraw


# A tiny pixel-art laptop the mascot pulls out mid-loop: 'B' = bezel/base,
# 'G' = the glowing screen, '.' = background.
LAPTOP_BITMAP = (
    "BBBBBBBB",
    "BGGGGGGB",
    "BGGGGGGB",
    "BBBBBBBB",
    ".BBBBBB.",
    "BBBBBBBB",
)


LAPTOP_BEZEL = "#4A3F4D"


LAPTOP_GLOW = "#FFE9C7"


def draw_laptop(image: Image.Image, top_left: tuple[int, int], cell_size: int) -> None:
    x0, y0 = top_left
    draw = ImageDraw.Draw(image)
    for row, line in enumerate(LAPTOP_BITMAP):
        for col, cell in enumerate(line):
            if cell == ".":
                continue
            fill = LAPTOP_BEZEL if cell == "B" else LAPTOP_GLOW
            x = x0 + col * cell_size
            y = y0 + row * cell_size
            draw.rectangle((x, y, x + cell_size - 1, y + cell_size - 1), fill=fill)


def laptop_geometry(mascot_w: int, cell_size: int = 3) -> tuple[int, int, int]:
    """(x, hidden_y, held_y) shared by the laptop and the typing-cursor overlay."""
    laptop_w = cell_size * len(LAPTOP_BITMAP[0])
    x = 10 + (mascot_w - laptop_w) // 2
    return x, 34, 21


# A tiny pixel-art mug the mascot pulls out mid-loop: 'B' = ceramic, 'C' = coffee.
MUG_BITMAP = (
    "BBBBBB",
    "BCCCCB",
    "BCCCCB",
    "BCCCCB",
    "BBBBBB",
)


MUG_CERAMIC = "#F5F4EF"


MUG_COFFEE = "#5A3A22"


STEAM_COLOR = "#C7C2BC"


def draw_mug(image: Image.Image, top_left: tuple[int, int], cell_size: int, steam_phase: int) -> None:
    x0, y0 = top_left
    draw = ImageDraw.Draw(image)
    for row, line in enumerate(MUG_BITMAP):
        for col, cell in enumerate(line):
            fill = MUG_CERAMIC if cell == "B" else MUG_COFFEE
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
        draw.rectangle((x, y, x + cell_size - 1, y + cell_size - 1), fill=STEAM_COLOR)


# A tiny "idea" spark for the "eureka" moment — a diamond burst, simplest
# shape that still reads clearly at a few pixels across.
BULB_BITMAP = (
    "..G..",
    ".GGG.",
    "GGGGG",
    ".GGG.",
    "..G..",
)


BULB_GLOW_ON = "#FFD966"


BULB_GLOW_OFF = "#6B5E3E"


def draw_bulb(image: Image.Image, top_left: tuple[int, int], cell_size: int, lit: bool) -> None:
    x0, y0 = top_left
    draw = ImageDraw.Draw(image)
    glow = BULB_GLOW_ON if lit else BULB_GLOW_OFF
    for row, line in enumerate(BULB_BITMAP):
        for col, cell in enumerate(line):
            if cell == ".":
                continue
            fill = glow
            x = x0 + col * cell_size
            y = y0 + row * cell_size
            draw.rectangle((x, y, x + cell_size - 1, y + cell_size - 1), fill=fill)
