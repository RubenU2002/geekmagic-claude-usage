"""A busy device is slow to answer a small request, not gone: connecting must be quick, the reply may take a while."""
import socket
import threading
import time
import unittest
from unittest.mock import patch

from geekmagic.adapters.device import client
from geekmagic.adapters.device.client import GeekMagicDevice


class _Slow:
    def __init__(self, delay):
        self.delay = delay
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
            conn.recv(65536)
            time.sleep(self.delay)
            try:
                conn.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
            except OSError:
                pass
            conn.close()


class SlowReplyTests(unittest.TestCase):
    def test_a_reply_slower_than_the_connect_timeout_is_still_taken(self):
        server = _Slow(1.0)
        self.addCleanup(server.sock.close)
        with patch.object(client, "CONNECT_TIMEOUT", 0.3):
            GeekMagicDevice(server.address).set_brightness(50)  # must not raise

    def test_a_reply_that_never_comes_gives_up(self):
        server = _Slow(3.0)
        self.addCleanup(server.sock.close)
        with patch.object(client, "REPLY_TIMEOUT", 0.5), self.assertRaises(OSError):
            GeekMagicDevice(server.address).set_brightness(50)


if __name__ == "__main__":
    unittest.main()
