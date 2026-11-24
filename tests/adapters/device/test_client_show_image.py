"""Showing an image on the screen takes one request (the device is slow), and the theme is only selected when needed."""
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse

from geekmagic.adapters.device import client
from geekmagic.adapters.device.client import GeekMagicDevice


class FakeDevice:
    """Answers /set?theme= and /set?img= like the SmallTV: OK for what it knows, FAIL otherwise."""

    def __init__(self, files=("a.gif",), refuse_img=0):
        self.requests, self.files, self.refuse_img = [], set(files), refuse_img
        device = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                url = urlparse(self.path)
                query = {k: v[0] for k, v in parse_qs(url.query).items()}
                device.requests.append(next(iter(query), ""))
                ok = "theme" in query or ("img" in query and query["img"].rsplit("/", 1)[-1] in device.files)
                if "img" in query and device.refuse_img > 0:
                    device.refuse_img, ok = device.refuse_img - 1, False
                body = b"OK" if ok else b"FAIL"
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        self.server = HTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.address = f"127.0.0.1:{self.server.server_port}"

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class ShowImageTests(unittest.TestCase):
    def setUp(self):
        client._theme_ready.clear()
        self.device = FakeDevice()
        self.addCleanup(self.device.close)

    def test_the_first_show_selects_the_theme_and_every_later_one_is_a_single_request(self):
        GeekMagicDevice(self.device.address).show_image("a.gif")
        self.assertEqual(self.device.requests, ["theme", "img"])
        GeekMagicDevice(self.device.address).show_image("a.gif")
        GeekMagicDevice(self.device.address).show_image("a.gif")
        self.assertEqual(self.device.requests, ["theme", "img", "img", "img"], "one request each from then on")

    def test_a_refused_image_reselects_the_theme_and_tries_once_more(self):
        GeekMagicDevice(self.device.address).show_image("a.gif")
        self.device.requests.clear()
        self.device.refuse_img = 1  # e.g. someone changed the theme on the device
        GeekMagicDevice(self.device.address).show_image("a.gif")
        self.assertEqual(self.device.requests, ["img", "theme", "img"])

    def test_an_image_the_device_does_not_have_is_an_error_and_forgets_the_theme(self):
        with self.assertRaisesRegex(OSError, "wouldn't show missing.gif"):
            GeekMagicDevice(self.device.address).show_image("missing.gif")
        self.assertNotIn(self.device.address, client._theme_ready)
        self.device.requests.clear()
        GeekMagicDevice(self.device.address).show_image("a.gif")
        self.assertEqual(self.device.requests, ["theme", "img"], "selected again, as it may have restarted")

    def test_an_unreachable_device_forgets_the_theme_too(self):
        GeekMagicDevice(self.device.address).show_image("a.gif")
        self.assertIn(self.device.address, client._theme_ready)
        self.device.close()
        with self.assertRaises(OSError):
            GeekMagicDevice(self.device.address).show_image("a.gif")
        self.assertNotIn(self.device.address, client._theme_ready)
        self.device = FakeDevice()  # (the cleanup closes it)

    def test_each_device_is_tracked_on_its_own(self):
        other = FakeDevice()
        self.addCleanup(other.close)
        GeekMagicDevice(self.device.address).show_image("a.gif")
        GeekMagicDevice(other.address).show_image("a.gif")
        self.assertEqual(other.requests, ["theme", "img"], "a second screen still gets its theme")


if __name__ == "__main__":
    unittest.main()
