"""PillowRenderer.to_png: the still picture of a GIF it drew."""
import unittest
from io import BytesIO

from PIL import Image

from geekmagic.adapters.render.palette import HEIGHT, WIDTH
from geekmagic.adapters.render.renderer import PillowRenderer
from tests.support import usage


class ToPngTests(unittest.TestCase):
    def test_a_rendered_screen_becomes_a_full_size_png(self):
        renderer = PillowRenderer()
        gif = renderer.render("claude", usage(current=42), "idle").gif
        png = renderer.to_png(gif)
        self.assertTrue(png.startswith(b"\x89PNG"))
        image = Image.open(BytesIO(png))
        self.assertEqual((image.size, image.mode), ((WIDTH, HEIGHT), "RGB"))


if __name__ == "__main__":
    unittest.main()
