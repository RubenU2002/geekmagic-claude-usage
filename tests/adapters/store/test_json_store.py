"""The state file: what was saved comes back, and a missing, damaged or unwritable file costs nothing but the memory."""
import tempfile
import unittest
from pathlib import Path

from geekmagic.adapters.store.json_store import JsonStateStore


class JsonStateStoreTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.path = self.dir / "state.json"
        self.store = JsonStateStore(self.path)

    def test_nothing_saved_yet_is_an_empty_state(self):
        self.assertEqual(self.store.load(), {})

    def test_a_damaged_file_is_an_empty_state(self):
        self.path.write_text("{not json", encoding="utf-8")
        self.assertEqual(self.store.load(), {})

    def test_a_file_that_is_not_an_object_is_an_empty_state(self):
        for text in ("[1, 2]", '"text"', "42", "null"):
            self.path.write_text(text, encoding="utf-8")
            self.assertEqual(self.store.load(), {}, text)

    def test_what_was_saved_comes_back(self):
        state = {"view": {"mode": "split"}, "alerts": {"claude": [80, 95]}, "name": "Acme"}
        self.store.save(state)
        self.assertEqual(self.store.load(), state)
        self.assertEqual(JsonStateStore(self.path).load(), state, "a new store on the same file sees it too")

    def test_saving_again_replaces_what_was_there(self):
        self.store.save({"a": 1})
        self.store.save({"b": 2})
        self.assertEqual(self.store.load(), {"b": 2})

    def test_a_path_that_cannot_be_written_does_not_raise(self):
        unwritable = JsonStateStore(self.dir / "missing folder" / "state.json")
        with self.assertLogs("tray", level="WARNING"):
            unwritable.save({"a": 1})
        self.assertEqual(unwritable.load(), {})


if __name__ == "__main__":
    unittest.main()
