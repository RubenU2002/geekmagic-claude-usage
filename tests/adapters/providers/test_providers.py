"""The providers the app ships with: each bundles how to read, count, watch and sign in, and hands the work to its own code."""
import unittest
from datetime import datetime
from unittest.mock import patch

from geekmagic.adapters.providers import claude, claude_logs, codex, codex_logs, default_providers
from geekmagic.adapters.render import animations
from geekmagic.core.model import Usage, Window
from geekmagic.core.ports import Provider
from geekmagic.core.registry import ProviderRegistry
from geekmagic.core.themes import THEMES


def reading(title):
    return Usage(title=title, current=Window(), weekly=Window(), now=datetime.now().astimezone())


class ProvidersTests(unittest.TestCase):
    def setUp(self):
        self.providers = default_providers()
        self.by_key = {p.key: p for p in self.providers}

    def test_every_provider_has_its_own_key_and_title_in_the_order_they_cycle(self):
        self.assertEqual([p.key for p in self.providers], ["claude", "codex"])
        registry = ProviderRegistry(self.providers)
        self.assertEqual(registry.titles, {"claude": "Claude", "codex": "Codex"})
        self.assertEqual(registry.key_of("Claude"), "claude", "a reading says who it is by title")
        self.assertEqual(registry.key_of("Codex"), "codex")

    def test_each_one_says_how_to_sign_in_and_where_to_get_its_cli(self):
        self.assertEqual({p.key: p.login_args for p in self.providers}, {"claude": ("auth", "login"), "codex": ("login",)})
        for provider in self.providers:
            self.assertTrue(provider.install_hint, provider.key)

    def test_each_one_has_a_look_and_animations_on_the_screen(self):
        for provider in self.providers:
            self.assertIn(provider.title, THEMES)
            self.assertIn(provider.title, animations.ANIMATION_GROUPS)
            self.assertIn(animations.WORKING_ANIMATION[provider.title], animations.ANIMATION_GROUPS[provider.title])
            self.assertIn(animations.WAITING_ANIMATION[provider.title], animations.ANIMATION_GROUPS[provider.title])

    def test_it_is_an_interface_nobody_can_use_half_done(self):
        with self.assertRaises(TypeError):
            Provider()

        class Incomplete(Provider):
            key, title = "x", "X"

        with self.assertRaises(TypeError):
            Incomplete()

    def test_reading_goes_to_each_providers_own_code(self):
        claude_usage, codex_usage = reading("Claude"), reading("Codex")
        with patch.object(claude, "fetch_usage", return_value=claude_usage) as read:
            self.assertIs(self.by_key["claude"].fetch(), claude_usage)
        read.assert_called_once()
        with patch.object(codex, "fetch_codex_usage", return_value=codex_usage) as read:
            self.assertIs(self.by_key["codex"].fetch(), codex_usage)
        read.assert_called_once()

    def test_counting_goes_to_each_providers_own_logs(self):
        with patch.object(claude_logs, "stats", return_value={"total": 1}) as count:
            self.assertEqual(self.by_key["claude"].local_stats(5.0), {"total": 1})
        count.assert_called_once_with(5.0)
        with patch.object(codex_logs, "stats", return_value=None) as count:
            self.assertIsNone(self.by_key["codex"].local_stats(5.0))
        count.assert_called_once_with(5.0)

    def test_watching_goes_to_each_providers_own_logs(self):
        with patch.object(codex_logs, "agent_state", return_value={"working": True}) as watch:
            self.assertEqual(self.by_key["codex"].agent_state(7.0), {"working": True})
        watch.assert_called_once_with(7.0)
        with patch.object(claude_logs, "agent_state", return_value={"working": False}) as watch:
            self.assertEqual(self.by_key["claude"].agent_state(7.0), {"working": False})
        watch.assert_called_once_with(7.0)

    def test_each_one_finds_its_own_cli(self):
        with patch.object(claude, "which", return_value="/bin/claude") as which:
            self.assertEqual(self.by_key["claude"].find_cli(), "/bin/claude")
        which.assert_called_once_with("claude")
        with patch.object(codex, "find_codex", return_value=None):
            self.assertIsNone(self.by_key["codex"].find_cli())
        with patch.object(codex, "find_codex", return_value="/bin/codex"):
            self.assertEqual(self.by_key["codex"].find_cli(), "/bin/codex")


if __name__ == "__main__":
    unittest.main()
