"""The DeviceLocator port is the discovery functions behind a class."""
import unittest
from unittest.mock import patch

from geekmagic.adapters.device import discovery
from geekmagic.adapters.device.discovery import Locator


class LocatorTests(unittest.TestCase):
    def test_probe_asks_the_module_function(self):
        with patch.object(discovery, "probe", return_value=True) as probe:
            self.assertTrue(Locator().probe("192.168.1.50", 0.5))
        probe.assert_called_once_with("192.168.1.50", 0.5)
        with patch.object(discovery, "probe", return_value=False) as probe:
            self.assertFalse(Locator().probe("192.168.1.50"))
        self.assertEqual(probe.call_args.args[0], "192.168.1.50")

    def test_find_device_asks_the_module_function(self):
        with patch.object(discovery, "find_device", return_value="192.168.1.51") as find:
            self.assertEqual(Locator().find_device("192.168.1.50"), "192.168.1.51")
        find.assert_called_once_with("192.168.1.50")
        with patch.object(discovery, "find_device", return_value=None) as find:
            self.assertIsNone(Locator().find_device())
        find.assert_called_once_with(None)


if __name__ == "__main__":
    unittest.main()
