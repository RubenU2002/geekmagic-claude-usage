"""The registry holds the providers in use, in the order they cycle, and answers who is who."""
import unittest

from geekmagic.core.ports import Provider
from geekmagic.core.registry import ProviderRegistry


class Tool(Provider):
    """The smallest working provider: it only has a name."""
    login_args = ("login",)
    install_hint = "Install it."

    def __init__(self, key: str, title: str) -> None:
        self.key, self.title = key, title

    def fetch(self):
        raise NotImplementedError

    def find_cli(self):
        return None

    def local_stats(self, now_epoch):
        return None

    def agent_state(self, now_epoch):
        return {}


class RegistryTests(unittest.TestCase):
    def setUp(self):
        self.alpha, self.beta, self.gamma = Tool("alpha", "Alpha AI"), Tool("beta", "Beta AI"), Tool("gamma", "Gamma AI")
        self.registry = ProviderRegistry([self.alpha, self.beta, self.gamma])

    def test_keys_come_in_the_order_the_providers_were_given(self):
        self.assertEqual(list(self.registry), ["alpha", "beta", "gamma"], "the order they cycle")
        self.assertEqual(list(self.registry.keys()), ["alpha", "beta", "gamma"])
        self.assertEqual(list(ProviderRegistry([self.gamma, self.alpha])), ["gamma", "alpha"])

    def test_each_provider_is_found_by_its_key(self):
        self.assertIs(self.registry["beta"], self.beta)
        self.assertEqual(list(self.registry.values()), [self.alpha, self.beta, self.gamma])
        self.assertEqual(dict(self.registry.items()), {"alpha": self.alpha, "beta": self.beta, "gamma": self.gamma})
        with self.assertRaises(KeyError):
            self.registry["nope"]

    def test_titles_by_key(self):
        self.assertEqual(self.registry.titles, {"alpha": "Alpha AI", "beta": "Beta AI", "gamma": "Gamma AI"})

    def test_a_reading_says_who_it_is_by_title(self):
        self.assertEqual(self.registry.key_of("Beta AI"), "beta")
        self.assertIsNone(self.registry.key_of("Nobody"))
        self.assertIsNone(self.registry.key_of(None))

    def test_length_and_membership(self):
        self.assertEqual(len(self.registry), 3)
        self.assertIn("alpha", self.registry)
        self.assertNotIn("Alpha AI", self.registry, "membership is by key, not by title")
        self.assertNotIn("nope", self.registry)

    def test_an_empty_registry(self):
        empty = ProviderRegistry([])
        self.assertEqual((len(empty), list(empty), empty.titles), (0, [], {}))
        self.assertIsNone(empty.key_of("Alpha AI"))

    def test_a_provider_is_known_by_its_repr(self):
        self.assertEqual(repr(self.alpha), "<Tool alpha>")


if __name__ == "__main__":
    unittest.main()
