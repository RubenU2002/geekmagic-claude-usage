"""The tray as the screen: with no GeekMagic to reach, the pictures are still drawn and kept, a click on the icon opens the
simulated screen, and a click inside it goes on to the next provider."""
import unittest

from geekmagic.application.local_screen import LocalScreen
from tests.support import AppTestCase, FakeRenderer, capture_screen, fake_fetchers, usage


class LocalScreenTests(unittest.TestCase):
    def test_it_remembers_the_picture_of_the_active_view_and_its_version_changes_with_it(self):
        active = ["claude"]
        local = LocalScreen(lambda: active[0])
        self.assertIsNone(local.latest(), "nothing drawn yet")
        local.remember("claude", b"one", "data-1")
        local.remember("codex", b"other", "data-2")
        view, first, gif = local.latest()
        self.assertEqual((view, gif), ("claude", b"one"))
        local.remember("claude", b"two", "data-3")
        self.assertGreater(local.latest()[1], first)
        active[0] = "codex"
        self.assertEqual(local.latest()[0::2], ("codex", b"other"))
        active[0] = "split"
        self.assertIsNone(local.latest(), "a view that was never drawn")

    def test_it_knows_when_a_picture_is_already_drawn_from_this_data(self):
        local = LocalScreen(lambda: "claude")
        self.assertFalse(local.is_current("claude", "x"))
        local.remember("claude", b"gif", "x")
        self.assertTrue(local.is_current("claude", "x"))
        self.assertFalse(local.is_current("claude", "y"))
        self.assertFalse(local.is_current("codex", "x"))


class CountingRenderer(FakeRenderer):
    def __init__(self) -> None:
        super().__init__()
        self.drawn = 0

    def render(self, key, data, animation):
        self.drawn += 1
        rendered = super().render(key, data, animation)
        rendered.gif = f"GIF-{self.drawn}".encode()
        return rendered


class OfflineDrawingTests(AppTestCase):
    def test_a_delivery_to_a_screen_that_is_there_also_keeps_the_picture(self):
        app = self.make()
        with fake_fetchers(claude=lambda: usage(10)):
            self.assertEqual(app.screen.deliver("claude", usage(10)), "ok")
        self.assertEqual(app.screen.local.latest()[0::2], ("claude", b"GIF89a"))

    def test_with_no_address_the_picture_is_still_drawn_for_the_simulated_screen(self):
        renderer = CountingRenderer()
        app = self.make(None, renderer=renderer)
        self.assertEqual(app.screen.ip, "")
        with capture_screen() as shown:
            self.assertEqual(app.screen.deliver("claude", usage(42)), "offline")
        self.assertEqual(shown.uploads, [], "nothing goes to a screen that isn't there")
        self.assertEqual(app.screen.local.latest()[0::2], ("claude", b"GIF-1"))

    def test_while_the_device_is_unreachable_a_retry_does_not_redraw_the_same_picture(self):
        renderer = CountingRenderer()
        app = self.make(renderer=renderer)
        app.screen.offline = True
        self.locator.reachable = False
        app.screen.slots["claude"] = "a"
        reading, newer = usage(42), usage(43)

        def no_answer(name):
            raise OSError("no answer")

        app.screen.device.show_image = no_answer
        for _ in range(3):  # the worker retries every few seconds with the very same reading
            self.assertEqual(app.screen.deliver("claude", reading), "offline")
        self.assertEqual(renderer.drawn, 1)
        self.assertEqual(app.screen.deliver("claude", newer), "offline")
        self.assertEqual(renderer.drawn, 2, "a new reading is drawn")
        app.agents.agent["claude"] = {"working": True}  # an agent started: the same reading now gets its "Working" tag
        app.screen.deliver("claude", newer)
        self.assertEqual(renderer.drawn, 3)

    def test_a_failure_to_draw_never_breaks_the_loop(self):
        class Broken(FakeRenderer):
            def render(self, key, data, animation):
                raise RuntimeError("cannot draw")

        app = self.make(None, renderer=Broken())
        self.assertEqual(app.screen.deliver("claude", usage(42)), "offline")
        self.assertIsNone(app.screen.local.latest())


class TrayClickTests(AppTestCase):
    def test_with_the_screen_there_a_click_goes_on_to_the_next_provider(self):
        app = self.make()
        app.tray_click()
        self.assertEqual(app.view.key, "codex")
        self.assertEqual(app.ui.simulator, [])

    def test_without_the_screen_the_click_opens_the_simulated_one_instead(self):
        app = self.make()
        app.screen.offline = True
        app.tray_click()
        self.assertEqual((app.ui.simulator, app.view.key), (["toggle"], "claude"), "the view didn't change")
        app.screen.offline = False
        app.tray_click()
        self.assertEqual(app.view.key, "codex")

    def test_with_no_address_at_all_it_also_opens_the_simulated_screen(self):
        app = self.make(None)
        app.tray_click()
        self.assertEqual(app.ui.simulator, ["toggle"])

    def test_the_menu_command_opens_it_whether_or_not_the_screen_is_there(self):
        app = self.make()
        app.open_simulator()
        self.assertEqual(app.ui.simulator, ["show"])


if __name__ == "__main__":
    unittest.main()
