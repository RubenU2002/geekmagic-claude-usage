"""The Renderer port drawn with Pillow: every view comes out as a GIF, and the rotation is remembered only when told."""
import unittest
from datetime import datetime
from io import BytesIO

from PIL import Image

from geekmagic.adapters.render.renderer import PillowRenderer
from geekmagic.adapters.render.version import RENDER_ID
from geekmagic.core.model import ErrorScreen
from geekmagic.core.ports import Rendered
from geekmagic.core.views import PANEL_KEYS
from tests.support import usage


def gif_size(rendered: Rendered) -> tuple[int, int]:
    image = Image.open(BytesIO(rendered.gif))
    assert image.format == "GIF"
    return image.size


def panels():
    return [usage(30, 10), usage(5, 2, title="Codex")]


class RenderTests(unittest.TestCase):
    def setUp(self):
        self.renderer = PillowRenderer()

    def test_every_view_of_two_providers_renders_to_a_gif(self):
        for key in PANEL_KEYS:
            with self.subTest(view=key):
                self.assertEqual(gif_size(self.renderer.render(key, panels(), "idle")), (240, 240))

    def test_a_providers_own_screen_renders_to_a_gif(self):
        self.assertEqual(gif_size(self.renderer.render("claude", usage(30, 10), "idle")), (240, 240))
        self.assertEqual(gif_size(self.renderer.render("codex", usage(30, 10, title="Codex"), "idle")), (240, 240))

    def test_a_screen_that_could_not_be_read_renders_to_a_gif(self):
        screen = ErrorScreen("Claude", "Claude could not be read.", datetime.now().astimezone())
        self.assertEqual(gif_size(self.renderer.render("claude", screen, "idle")), (240, 240))

    def test_the_version_is_the_one_the_render_code_computed(self):
        self.assertEqual(self.renderer.version, RENDER_ID)
        self.assertEqual(PillowRenderer().version, self.renderer.version)


class RotationTests(unittest.TestCase):
    def setUp(self):
        self.renderer = PillowRenderer()

    def test_a_pick_is_remembered_only_once_confirmed(self):
        rendered = self.renderer.render("claude", usage(30, 10), "random")
        self.assertEqual(len(rendered.picks), 1)
        title, name = rendered.picks[0]
        self.assertEqual(title, "Claude")
        self.assertEqual(self.renderer.rotation.recent, {}, "a render alone is not on the device yet")
        self.renderer.confirm(rendered)
        self.assertEqual(self.renderer.rotation.recent, {"Claude": [name]})

    def test_a_fixed_animation_is_not_a_pick(self):
        rendered = self.renderer.render("claude", usage(30, 10), "idle")
        self.assertEqual(rendered.picks, [])
        self.renderer.confirm(rendered)
        self.assertEqual(self.renderer.rotation.recent, {})

    def test_both_providers_of_a_view_of_two_are_confirmed(self):
        rendered = self.renderer.render("split", panels(), "random")
        self.assertEqual({title for title, _ in rendered.picks}, {"Claude", "Codex"})
        self.renderer.confirm(rendered)
        self.assertEqual(set(self.renderer.rotation.recent), {"Claude", "Codex"})

    def test_what_it_remembers_survives_a_restart(self):
        self.renderer.confirm(self.renderer.render("claude", usage(30, 10), "random"))
        saved = self.renderer.snapshot()
        self.assertTrue(saved["animations"])
        again = PillowRenderer()
        again.restore(saved)
        self.assertEqual(again.snapshot(), saved)
        self.assertEqual(again.rotation.recent, self.renderer.rotation.recent)

    def test_restoring_nothing_or_garbage_is_harmless(self):
        self.renderer.restore({})
        self.renderer.restore({"animations": "garbage"})
        self.assertEqual(self.renderer.snapshot(), {"animations": {}})


if __name__ == "__main__":
    unittest.main()
