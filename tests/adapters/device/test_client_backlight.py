"""Brightness and night mode: the calls the device's own settings page makes, against a fake device."""
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse

from geekmagic.adapters.device.client import GeekMagicDevice


class FakeDevice:
    """Answers OK to the /set settings it knows and FAIL to the rest, like the real SmallTV."""

    KNOWN = ({"brt"}, {"t1", "t2", "b1", "b2", "en"})

    def __init__(self, refuse=False):
        self.requests = []
        device = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                url = urlparse(self.path)
                params = {k: v[0] for k, v in parse_qs(url.query).items()}
                device.requests.append((url.path, params))
                ok = not refuse and url.path == "/set" and any(set(params) == known for known in device.KNOWN)
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


class BrightnessTests(unittest.TestCase):
    def setUp(self):
        self.device = FakeDevice()
        self.addCleanup(self.device.close)

    def test_set_brightness_sends_brt(self):
        GeekMagicDevice(self.device.address).set_brightness(75)
        self.assertEqual(self.device.requests, [("/set", {"brt": "75"})])

    def test_levels_are_clamped_to_what_the_device_accepts(self):
        for asked, sent in ((500, "100"), (-99, "-10"), (-10, "-10"), (0, "0"), (49.9, "49")):
            GeekMagicDevice(self.device.address).set_brightness(asked)
            self.assertEqual(self.device.requests[-1][1], {"brt": sent}, asked)

    def test_night_mode_sends_the_schedule_the_way_the_settings_page_does(self):
        GeekMagicDevice(self.device.address).set_night_mode(start_hour=22, end_hour=7, night_level=10, day_level=60, enabled=True)
        self.assertEqual(self.device.requests, [("/set", {"t1": "22", "t2": "7", "b1": "60", "b2": "10", "en": "1"})])
        GeekMagicDevice(self.device.address).set_night_mode(start_hour=24, end_hour=31, night_level=200, day_level=-50, enabled=False)
        self.assertEqual(self.device.requests[-1][1], {"t1": "0", "t2": "7", "b1": "-10", "b2": "100", "en": "0"})

    def test_a_refusal_is_an_error_not_silence(self):
        refusing = FakeDevice(refuse=True)
        self.addCleanup(refusing.close)
        with self.assertRaisesRegex(OSError, "refused the setting \\(FAIL\\)"):
            GeekMagicDevice(refusing.address).set_brightness(50)
        with self.assertRaises(OSError):
            GeekMagicDevice(refusing.address).set_night_mode(start_hour=22, end_hour=7, night_level=10, day_level=50, enabled=True)

    def test_an_unreachable_device_is_an_oserror(self):
        with self.assertRaises(OSError):
            GeekMagicDevice("127.0.0.1:1").set_brightness(50)


if __name__ == "__main__":
    unittest.main()
