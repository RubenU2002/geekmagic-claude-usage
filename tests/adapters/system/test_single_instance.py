import tempfile
import unittest
from pathlib import Path

from geekmagic.adapters.system import single_instance


class SingleInstanceTests(unittest.TestCase):
    def setUp(self):
        self.path = Path(tempfile.mkdtemp()) / "app.lock"

    def test_a_second_copy_is_refused_while_the_first_runs(self):
        first = single_instance.acquire(self.path)
        self.addCleanup(single_instance.release, first)
        with self.assertRaises(single_instance.AlreadyRunning):
            single_instance.acquire(self.path)

    def test_the_lock_frees_up_when_the_first_lets_go(self):
        single_instance.release(single_instance.acquire(self.path))
        again = single_instance.acquire(self.path)  # must not raise
        single_instance.release(again)


if __name__ == "__main__":
    unittest.main()
