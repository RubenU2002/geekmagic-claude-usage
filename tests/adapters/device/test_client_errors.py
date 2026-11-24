"""A device that cuts responses short is reported as "didn't answer", so the tray retries soon."""
import http.client
import logging
import threading
import unittest

from geekmagic.adapters.device.client import GeekMagicDevice
from tests.support import AppTestCase, usage


class _Truncating:
    """A server that promises a body and hangs up half way, the way the SmallTV does when it's overloaded."""

    def __init__(self):
        import socket
        self.sock = socket.socket()
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(5)
        self.address = f"127.0.0.1:{self.sock.getsockname()[1]}"
        threading.Thread(target=self._serve, daemon=True).start()

    def _serve(self):
        while True:
            try:
                conn, _ = self.sock.accept()
            except OSError:
                return
            conn.settimeout(2)
            try:
                conn.recv(65536)
                conn.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\n\r\nonly a bit")
            except OSError:
                pass
            finally:
                conn.close()

    def close(self):
        self.sock.close()


class DeviceErrorTests(unittest.TestCase):
    def setUp(self):
        logging.disable(logging.CRITICAL)  # keep the app's log lines off the console
        self.addCleanup(logging.disable, logging.NOTSET)
        self.server = _Truncating()
        self.addCleanup(self.server.close)

    def test_truncated_responses_become_oserrors(self):
        for call in (
            lambda: GeekMagicDevice(self.server.address).show_image("x.gif"),
            lambda: GeekMagicDevice(self.server.address).list_images(),
            lambda: GeekMagicDevice(self.server.address).delete_image("x.gif"),
        ):
            with self.assertRaises(OSError) as ctx:
                call()
            self.assertIn("incomplete or invalid response", str(ctx.exception))

    def test_an_upload_the_device_hangs_up_on_is_an_oserror_too(self):
        # here the server resets the connection (an OSError already) or cuts the reply (wrapped): either way
        with self.assertRaises(OSError):
            GeekMagicDevice(self.server.address).upload(b"GIF89a", "x.gif")

    def test_the_underlying_error_really_is_not_an_oserror(self):
        # why the wrapper exists: without it this slips past `except OSError`
        with self.assertRaises(http.client.HTTPException):
            import urllib.request
            urllib.request.urlopen(f"http://{self.server.address}/x", timeout=2).read()


class TrayDeviceErrorTests(AppTestCase):
    def setUp(self):
        super().setUp()
        self.server = _Truncating()
        self.addCleanup(self.server.close)

    def test_the_tray_treats_it_as_the_device_being_unreachable(self):
        app = self.make(ip=self.server.address, display_factory=GeekMagicDevice)  # the real client, against the broken server
        outcome = app.screen.deliver("claude", usage(1.0, 1.0))
        self.assertEqual(outcome, "offline")
        self.assertTrue(app.screen.offline)


if __name__ == "__main__":
    unittest.main()
