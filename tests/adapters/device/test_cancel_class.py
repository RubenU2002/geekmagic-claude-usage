"""The device adapter and the application must agree on what a cancelled upload is."""
import threading
import unittest

from geekmagic.adapters.device import client
from geekmagic.adapters.device.client import GeekMagicDevice
from geekmagic.core import errors


class CancelledUploadTests(unittest.TestCase):
    def test_the_adapter_raises_the_core_error(self):
        self.assertIs(client.UploadCancelled, errors.UploadCancelled)

    def test_a_cancelled_upload_raises_it(self):
        cancel = threading.Event()
        cancel.set()
        with self.assertRaises(errors.UploadCancelled):
            GeekMagicDevice("127.0.0.1:1").upload(b"GIF89a", "x.gif", cancel=cancel)


if __name__ == "__main__":
    unittest.main()
