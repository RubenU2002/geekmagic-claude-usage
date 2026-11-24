"""The screenshot: ask where, read back what the screen shows, save it as PNG or as the animated GIF."""
from geekmagic.application import image_files
from tests.support import AppTestCase


class ScreenCaptureTests(AppTestCase):
    def ready(self):
        app = self.make("192.168.1.50")
        app.screen.slots["claude"] = "b"
        app.view.provider, app.view.mode = "claude", None
        self.target = self.tmp / "shot.png"
        app.ui.save_to = str(self.target)
        return app

    def test_saves_what_the_screen_shows_as_a_png(self):
        app = self.ready()
        path = app.capture.capture()
        self.assertEqual(path, self.target)
        self.assertEqual(self.target.read_bytes(), b"PNG-of-GIF89a-of-" + image_files.slot_file("claude", "b").encode())
        self.assertEqual(self.notes[-1], ("Pantallazo guardado", str(self.target)))
        self.assertIn(("download", image_files.slot_file("claude", "b")), self.displays[-1].calls)

    def test_a_gif_name_keeps_it_animated(self):
        app = self.ready()
        self.target = self.tmp / "shot.GIF"
        app.ui.save_to = str(self.target)
        app.capture.capture()
        self.assertTrue(self.target.read_bytes().startswith(b"GIF89a-of-"))

    def test_the_suggested_name_says_which_view(self):
        app = self.ready()
        app.capture.capture()
        self.assertRegex(app.ui.asked_names[-1], r"^geekmagic-claude-\d{8}-\d{6}\.png$")

    def test_cancelling_the_dialog_saves_nothing_and_says_nothing(self):
        app = self.ready()
        app.ui.save_to = None
        self.assertIsNone(app.capture.capture())
        self.assertEqual((self.notes, self.displays[-1].calls), ([], []))

    def test_without_an_answering_screen_it_says_so_before_asking(self):
        app = self.ready()
        app.screen.offline = True
        self.assertIsNone(app.capture.capture())
        self.assertEqual(app.ui.asked_names, [])
        self.assertIn("no responde", self.notes[-1][1])

    def test_nothing_stored_for_the_view_yet(self):
        app = self.ready()
        app.screen.slots.clear()
        self.assertIsNone(app.capture.capture())
        self.assertIn("todavía no hay una imagen", self.notes[-1][1])

    def test_a_device_failure_is_reported_and_leaves_no_file(self):
        app = self.ready()
        def boom(filename):
            raise OSError("timed out")
        self.displays[-1].download_image = boom
        self.assertIsNone(app.capture.capture())
        self.assertFalse(self.target.exists())
        self.assertIn("timed out", self.notes[-1][1])
