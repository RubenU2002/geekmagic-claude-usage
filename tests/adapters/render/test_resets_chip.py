"""The note in the corner of Codex's screen about its free rate-limit resets."""
import unittest
from datetime import datetime

from geekmagic.adapters.render import components, palette
from geekmagic.core.model import CodexExtra, Usage, Window
from geekmagic.core.themes import THEMES


class ResetsChipTests(unittest.TestCase):
    def usage(self, title="Codex", codex_extra=None):
        return Usage(title=title, current=Window(), weekly=Window(), now=datetime.now().astimezone(), codex_extra=codex_extra)

    def chip(self, **codex_extra):
        return components.resets_chip(self.usage(codex_extra=CodexExtra(**codex_extra)))

    def test_chip_text_and_urgency(self):
        self.assertEqual(self.chip(free_resets=3, next_reset_expires_days=12), ("3 resets · 12d", THEMES["Codex"]["weekly"]))
        self.assertEqual(self.chip(free_resets=1, next_reset_expires_days=3), ("1 reset · 3d", palette.STATE_COLORS["warn"]))
        self.assertEqual(self.chip(free_resets=2, next_reset_expires_days=1), ("2 resets · 1d", palette.STATE_COLORS["crit"]))
        self.assertEqual(self.chip(free_resets=2, next_reset_expires_days=0)[1], palette.STATE_COLORS["crit"])
        self.assertEqual(self.chip(free_resets=2, next_reset_expires_days=None)[0], "2 resets")

    def test_no_chip_without_resets_or_for_claude(self):
        self.assertIsNone(self.chip(free_resets=0, next_reset_expires_days=None))
        self.assertIsNone(components.resets_chip(self.usage()))
        self.assertIsNone(components.resets_chip(self.usage("Claude", CodexExtra(free_resets=3))))


if __name__ == "__main__":
    unittest.main()
