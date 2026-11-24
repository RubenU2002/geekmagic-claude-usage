"""The things that make the tray respond quickly: reading both providers at once, and what goes up first at start-up."""
import json
import threading
import time
import unittest

from geekmagic.application import config
from geekmagic.core.errors import UsageError
from geekmagic.core.views import SPLIT
from tests.support import AppTestCase, FakeDisplay, FakeRenderer, capture_screen, fake_fetchers, usage


class ParallelReadTests(AppTestCase):
    """The tray reads Claude and Codex at the same time."""

    def test_both_providers_are_read_at_the_same_time(self):
        app = self.make()
        started = {"claude": threading.Event(), "codex": threading.Event()}

        def read(provider):
            def run():
                started[provider].set()
                other = "codex" if provider == "claude" else "claude"
                # each waits for the other to have started: sequential reads would time out here
                self.assertTrue(started[other].wait(3), f"{other} was not read while {provider} was")
                return usage(title="Claude" if provider == "claude" else "Codex")
            return run

        with fake_fetchers(claude=read("claude"), codex=read("codex")):
            t = time.monotonic()
            app.scheduler.fetch_all()
            took = time.monotonic() - t
        self.assertEqual(sorted(app.usage.last_good), ["claude", "codex"])
        self.assertLess(took, 2)

    def test_the_view_update_takes_as_long_as_the_slower_read_not_both(self):
        app = self.make()

        def slow(title):
            def read():
                time.sleep(0.4)
                return usage(title=title)
            return read

        with fake_fetchers(claude=slow("Claude"), codex=slow("Codex")), capture_screen():
            app.view.mode = SPLIT
            t = time.monotonic()
            self.assertEqual(app.scheduler.update_split(), "ok")
            took = time.monotonic() - t
        self.assertLess(took, 0.75, "two 0.4 s reads, in parallel")

    def test_a_failing_read_does_not_hold_up_or_break_the_other(self):
        app = self.make()

        def broken():
            raise UsageError("not logged in")

        with fake_fetchers(claude=broken, codex=lambda: usage(title="Codex")):
            app.scheduler.fetch_all()
        self.assertEqual(sorted(app.usage.last_good), ["codex"])

    def test_saving_state_while_readings_arrive_in_other_threads_never_breaks(self):
        app = self.make()
        errors, stop = [], threading.Event()

        def hammer_saves():
            while not stop.is_set():
                try:
                    app.persistence.save()
                except Exception as e:  # e.g. "dictionary changed size during iteration"
                    errors.append(e)

        counter = iter(range(10**6))

        def hammer_readings(provider):
            for _ in range(60):
                app.usage.fetch(provider)

        # (one patch for all the readings, not one per reading)
        with fake_fetchers(claude=lambda: usage(next(counter) % 99, title="Claude"),
                           codex=lambda: usage(next(counter) % 99, title="Codex")):
            saver = threading.Thread(target=hammer_saves)
            saver.start()
            readers = [threading.Thread(target=hammer_readings, args=(p,)) for p in ("claude", "codex")]
            for r in readers:
                r.start()
            for r in readers:
                r.join()
            stop.set()
            saver.join()
        self.assertEqual(errors, [])
        saved = json.loads(self.store.path.read_text())
        self.assertIn("claude/current", saved["history"])

    def test_agents_are_polled_often_enough_to_feel_live(self):
        self.assertLessEqual(config.AGENT_POLL, 2)


class StartupOrderTests(AppTestCase):
    def remember_a_view(self):
        """A state file as the last run left it: its image is on the device and the code that drew it is current."""
        self.store.path.write_text(json.dumps(
            {"slots": {"claude": "a"}, "view": "claude", "render": FakeRenderer.version, "ip": "192.168.1.20"}))

    def start_up(self, show, probe, list_images):
        """Run only the start-up part of the scheduler's loop (the loop itself is told to stop at once). The screen's
        requests go through the given functions so a test can tell in what order they were made."""
        def display(address):
            fake = FakeDisplay(address)
            fake.show_image = show
            fake.list_images = list_images
            return fake

        self.locator.probe = probe
        app = self.make(ip=None, display_factory=display)
        app.stop.set()
        app.scheduler.run()

    def test_the_remembered_view_goes_up_before_the_search_and_the_file_list(self):
        self.remember_a_view()
        order = []
        self.start_up(show=lambda name: order.append("show"),
                      probe=lambda *a, **k: order.append("search") or True,
                      list_images=lambda: order.append("files") or {"claude-usage-a.gif"})
        self.assertEqual(order[:3], ["show", "search", "files"])
        self.assertEqual(order.count("show"), 1, "and not shown a second time when the first got through")

    def test_it_is_shown_again_if_the_first_try_did_not_get_through(self):
        self.remember_a_view()
        attempts = []

        def flaky(name):
            attempts.append(name)
            if len(attempts) == 1:
                raise OSError("timed out")

        self.start_up(show=flaky, probe=lambda *a, **k: True, list_images=lambda: {"claude-usage-a.gif"})
        self.assertEqual(len(attempts), 2)


if __name__ == "__main__":
    unittest.main()
