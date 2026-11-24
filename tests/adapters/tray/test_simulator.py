"""The simulated screen's window: it plays the GIF it is given, follows a new picture, forwards a click, and closes. It opens
a real (briefly visible) Tk window, so the tests that need one are skipped where there is no desktop."""
import logging
import time
import unittest
from io import BytesIO

from PIL import Image

from geekmagic.adapters.tray.simulator import SIDE, SimulatorWindow, decode_frames


def gif(colors, size=SIDE) -> bytes:
    """A looping GIF with one solid frame per colour."""
    frames = [Image.new("RGB", (size, size), color) for color in colors]
    out = BytesIO()
    frames[0].save(out, format="GIF", save_all=True, append_images=frames[1:], duration=130, loop=0)
    return out.getvalue()


def wait_for(condition, seconds=10.0) -> bool:
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if condition():
            return True
        time.sleep(0.05)
    return False


class DecodeTests(unittest.TestCase):
    def test_every_frame_comes_out_complete(self):
        frames = decode_frames(gif([(255, 0, 0), (0, 0, 255)]))
        self.assertEqual(len(frames), 2)
        self.assertEqual([f.getpixel((5, 5)) for f in frames], [(255, 0, 0), (0, 0, 255)])
        self.assertEqual(frames[0].size, (SIDE, SIDE))


def tk_available() -> bool:
    try:
        import tkinter
        root = tkinter.Tk()
        root.destroy()
        return True
    except Exception:
        return False


@unittest.skipUnless(tk_available(), "no desktop to open a window on")
class WindowTests(unittest.TestCase):
    def setUp(self):
        logging.disable(logging.CRITICAL)  # one test feeds it a broken picture on purpose
        self.addCleanup(logging.disable, logging.NOTSET)

    def make(self, current):
        clicks, titles = [], []
        window = SimulatorWindow(lambda: current[0], lambda: clicks.append(True), lambda: titles.append(1) or "GeekMagic (simulada)")
        self.addCleanup(window.close)
        return window, clicks, titles

    def test_it_opens_shows_the_picture_forwards_a_click_and_toggles_closed(self):
        current = [("claude", 1, gif([(250, 84, 7), (24, 22, 15)]))]
        window, clicks, titles = self.make(current)
        self.assertFalse(window.is_open)
        window.toggle()
        self.assertTrue(window.is_open)
        self.assertTrue(wait_for(lambda: len(titles) > 5), "it is ticking: playing the frames")
        window._requests.put("click")
        self.assertTrue(wait_for(lambda: clicks), "a click in the window reaches the app")
        window.toggle()
        self.assertTrue(wait_for(lambda: not window.is_open), "a second toggle closes it")

    def test_a_new_picture_replaces_the_old_one_and_a_window_with_nothing_to_show_waits(self):
        current = [None]
        window, _, titles = self.make(current)
        window.show()
        self.assertTrue(wait_for(lambda: len(titles) > 2), "it opens even with nothing to show yet")
        current[0] = ("claude", 1, gif([(255, 0, 0)]))
        seen = len(titles)
        self.assertTrue(wait_for(lambda: len(titles) > seen + 2), "and keeps going when the picture arrives")
        current[0] = ("claude", 2, b"not a gif")  # a broken picture must not take the window down
        seen = len(titles)
        self.assertTrue(wait_for(lambda: len(titles) > seen + 2))
        self.assertTrue(window.is_open)
        window.close()
        self.assertTrue(wait_for(lambda: not window.is_open))

    def test_it_can_be_opened_again_after_it_was_closed(self):
        window, _, _ = self.make([("claude", 1, gif([(1, 2, 3)]))])
        for _ in range(2):
            window.show()
            self.assertTrue(wait_for(lambda: window.is_open))
            window.close()
            self.assertTrue(wait_for(lambda: not window.is_open))


if __name__ == "__main__":
    unittest.main()
