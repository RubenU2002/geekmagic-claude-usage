"""Encoding frames as the small looping GIF the device stores."""

from __future__ import annotations

from io import BytesIO

from PIL import Image

from geekmagic.adapters.render.palette import HEIGHT, WIDTH


def encode_gif(frames: list[Image.Image]) -> bytes:
    # One palette built from *all* frames, so colours that only appear later (a bolt, sparkles, steam) survive.
    sheet = Image.new("RGB", (WIDTH, HEIGHT * len(frames)))
    for i, frame in enumerate(frames):
        sheet.paste(frame, (0, i * HEIGHT))
    base = sheet.quantize(colors=96)
    quantized = [f.quantize(palette=base) for f in frames]
    buf = BytesIO()
    # disposal=1 ("leave the frame in place") lets each frame after the first be stored as just the rectangle that
    # changed: the screens are static except for a mascot, so the GIFs shrink 5-10x (a 118 KB split view is 22 KB).
    # The device takes ~36 ms per KB to store an upload, so this is most of the time an update used to take.
    quantized[0].save(
        buf, format="GIF", save_all=True, append_images=quantized[1:],
        duration=130, loop=0, disposal=1, optimize=False,
    )
    return buf.getvalue()
