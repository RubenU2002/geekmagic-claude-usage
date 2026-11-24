"""Small GIFs: the screens are stored on a slow device, so what is uploaded has to be tiny, and still exactly what was drawn."""
import io
import unittest
from datetime import datetime, timedelta

from PIL import Image

from geekmagic.core.model import Usage, Window
from geekmagic.adapters.render import animations
from geekmagic.adapters.render import gif
from geekmagic.adapters.render import palette
from geekmagic.adapters.render.views import panels as panel_views
from geekmagic.adapters.render.views import single
from geekmagic.adapters.render.views import split


def reading(title="Claude", pct=36.0, **extra) -> Usage:
    now = datetime.now().astimezone()
    return Usage(title=title, current=Window(pct, now + timedelta(minutes=103)),
                 weekly=Window(17.0, now + timedelta(days=5)), now=now, **extra)


class GifSizeTests(unittest.TestCase):
    def frames(self, animation="idle"):
        u = reading()
        return [single.render_frame(u, dx, dy, extra) for dx, dy, extra in animations.ANIMATIONS[animation]]

    def full_frame_size(self, frames):
        """What the same frames cost when every one is stored whole (how the GIFs used to be written)."""
        sheet = Image.new("RGB", (palette.WIDTH, palette.HEIGHT * len(frames)))
        for i, f in enumerate(frames):
            sheet.paste(f, (0, i * palette.HEIGHT))
        base = sheet.quantize(colors=96)
        quantized = [f.quantize(palette=base) for f in frames]
        buf = io.BytesIO()
        quantized[0].save(buf, format="GIF", save_all=True, append_images=quantized[1:], duration=130, loop=0, disposal=2, optimize=False)
        return len(buf.getvalue())

    def test_animations_are_several_times_smaller_than_storing_every_frame_whole(self):
        for animation in ("idle", "typing", "coffee"):
            frames = self.frames(animation)
            small, whole = len(gif.encode_gif(frames)), self.full_frame_size(frames)
            self.assertLess(small * 3, whole, f"{animation}: {small} B vs {whole} B")

    def test_a_split_view_upload_is_small_enough_to_store_in_about_a_second(self):
        panels = [reading(), reading("Codex")]
        data = split.render_split(panels, ["laptop", "bolt"])
        # the device stores ~36 ms per KB: under ~30 KB is under ~1.1 s (it used to be ~120 KB, 4.3 s)
        self.assertLess(len(data), 40 * 1024)

    def test_the_stills_are_tiny(self):
        panels = [reading(activity=None), reading("Codex", activity=None)]
        self.assertLess(len(panel_views.render_stats(panels)), 20 * 1024)


class GifFidelityTests(unittest.TestCase):
    """Smaller must not mean different: every frame still decodes to exactly what was drawn."""

    def test_every_frame_decodes_to_the_frame_that_was_drawn(self):
        u = reading()
        frames = [single.render_frame(u, dx, dy, extra) for dx, dy, extra in animations.ANIMATIONS["typing"]]
        decoded = Image.open(io.BytesIO(gif.encode_gif(frames)))
        # what each frame looks like after the shared palette (the only loss, and the same as before)
        sheet = Image.new("RGB", (palette.WIDTH, palette.HEIGHT * len(frames)))
        for i, f in enumerate(frames):
            sheet.paste(f, (0, i * palette.HEIGHT))
        base = sheet.quantize(colors=96)
        expected = [f.quantize(palette=base).convert("RGB") for f in frames]
        shown = []
        for i in range(decoded.n_frames):
            decoded.seek(i)
            shown.append(decoded.convert("RGB"))
        # identical frames in a row are merged into one longer frame, so compare as a timeline
        timeline = []
        for i in range(decoded.n_frames):
            decoded.seek(i)
            timeline += [shown[i]] * max(1, round(decoded.info.get("duration", 130) / 130))
        self.assertEqual(len(timeline), len(expected))
        for i, (got, want) in enumerate(zip(timeline, expected)):
            self.assertEqual(got.tobytes(), want.tobytes(), f"frame {i}")

    def test_it_loops_forever_and_keeps_the_frame_timing(self):
        data = gif.encode_gif([single.render_frame(reading(), dx, dy, extra) for dx, dy, extra in animations.ANIMATIONS["dance"]])
        self.assertTrue(data.startswith(b"GIF89a"))
        self.assertIn(b"NETSCAPE2.0", data, "the loop extension")
        im = Image.open(io.BytesIO(data))
        self.assertEqual(im.info.get("loop"), 0)
        self.assertEqual(im.size, (palette.WIDTH, palette.HEIGHT))
        total = 0
        for i in range(im.n_frames):
            im.seek(i)
            total += im.info.get("duration", 0)
        self.assertEqual(total, 130 * len(animations.ANIMATIONS["dance"]))

    def test_the_first_frame_is_the_whole_screen_and_later_ones_stay_inside_it(self):
        im = Image.open(io.BytesIO(gif.encode_gif([single.render_frame(reading(), dx, dy, extra) for dx, dy, extra in animations.ANIMATIONS["laptop"]])))
        for i in range(im.n_frames):
            im.seek(i)
            x0, y0, x1, y1 = im.tile[0][1]
            self.assertTrue(0 <= x0 < x1 <= palette.WIDTH and 0 <= y0 < y1 <= palette.HEIGHT, f"frame {i}: {im.tile[0][1]}")
            if i == 0:
                self.assertEqual((x0, y0, x1, y1), (0, 0, palette.WIDTH, palette.HEIGHT), "the loop restarts on a complete picture")
            self.assertEqual(im.disposal_method, 1)

    def test_a_single_frame_still_encodes(self):
        data = gif.encode_gif([single.render_frame(reading())])
        self.assertEqual(Image.open(io.BytesIO(data)).n_frames, 1)


if __name__ == "__main__":
    unittest.main()
