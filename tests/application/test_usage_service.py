"""Reading the providers and marking the data stale when it can't be refreshed."""
import time

from geekmagic.application import config
from geekmagic.core.errors import UsageError
from tests.support import AppTestCase, capture_screen, failing, fake_fetchers, usage


class UsageServiceTests(AppTestCase):
    def test_stale_image_after_a_few_minutes_without_fresh_data(self):
        app = self.make()
        with capture_screen() as cap:
            app.usage.last_good["claude"] = (usage(current=40), time.monotonic() - 60)
            app.scheduler.mark_stale("claude")
            self.assertEqual(cap.single, [], "only a minute old: still fine")

            app.usage.last_good["claude"] = (usage(current=40), time.monotonic() - config.STALE_SECONDS - 5)
            app.scheduler.mark_stale("claude")
            self.assertEqual(len(cap.single), 1)
            shown = cap.single[0][0]
            self.assertTrue(shown.stale)
            self.assertEqual(shown.current.pct, 40)  # the last known numbers, not made-up ones

            app.scheduler.mark_stale("claude")
            self.assertEqual(len(cap.single), 1, "already marked: don't re-upload every cycle")

            with fake_fetchers(claude=lambda: usage(current=41)):
                self.assertEqual(app.usage.fetch("claude").current.pct, 41)
            self.assertNotIn("claude", app.usage.stale)  # fresh data clears it

    def test_failing_fetch_marks_stale_and_returns_nothing(self):
        app = self.make()
        app.usage.last_good["claude"] = (usage(current=40), time.monotonic() - config.STALE_SECONDS - 5)
        with fake_fetchers(claude=failing(UsageError("claude not logged in"))), capture_screen() as cap:
            self.assertIsNone(app.usage.fetch("claude"))
        self.assertEqual(len(cap.single), 1)
        self.assertTrue(cap.single[0][0].stale)

    def test_no_stale_upload_while_the_device_is_unreachable(self):
        app = self.make()
        app.screen.offline = True
        app.usage.last_good["claude"] = (usage(current=40), time.monotonic() - 1000)
        with capture_screen() as cap:
            app.scheduler.mark_stale("claude")
        self.assertEqual((cap.single, cap.uploads), ([], []))


class BackoffTests(AppTestCase):
    def setUp(self):
        super().setUp()
        self.calls = 0

    def read(self):
        self.calls += 1
        raise UsageError("timed out")

    def fail_times(self, app, n):
        with fake_fetchers(claude=self.read):
            for _ in range(n):
                self.assertIsNone(app.usage.fetch("claude"))

    def test_keeps_reading_for_the_first_failures(self):
        app = self.make()
        self.fail_times(app, config.BACKOFF_FROM - 1)
        self.assertEqual(self.calls, config.BACKOFF_FROM - 1)
        self.assertNotIn("claude", app.usage.retry_at)

    def test_leaves_a_provider_that_keeps_failing_alone(self):
        app = self.make()
        self.fail_times(app, config.BACKOFF_FROM + 4)
        self.assertEqual(self.calls, config.BACKOFF_FROM, "the rest were skipped without calling the CLI")

    def test_skipped_reads_still_report_the_failure(self):
        app = self.make()
        seen = []
        app.usage.on_failure = lambda provider, error: seen.append(str(error))
        self.fail_times(app, config.BACKOFF_FROM + 2)
        self.assertEqual(len(seen), config.BACKOFF_FROM + 2)

    def test_tries_again_when_the_wait_is_over(self):
        app = self.make()
        self.fail_times(app, config.BACKOFF_FROM)
        app.usage.retry_at["claude"] = time.monotonic() - 1
        self.fail_times(app, 1)
        self.assertEqual(self.calls, config.BACKOFF_FROM + 1)

    def test_the_wait_grows_up_to_a_limit(self):
        app = self.make()
        waits = []
        with fake_fetchers(claude=self.read):
            for _ in range(config.BACKOFF_FROM + 8):
                app.usage.retry_at.clear()
                before = time.monotonic()
                app.usage.fetch("claude")
                if "claude" in app.usage.retry_at:
                    waits.append(round(app.usage.retry_at["claude"] - before))
        self.assertEqual(waits[0], config.BACKOFF_BASE)
        self.assertEqual(waits[1], min(config.BACKOFF_BASE * 2, config.BACKOFF_MAX))
        self.assertEqual(max(waits), config.BACKOFF_MAX)

    def test_a_good_read_forgets_the_failures(self):
        app = self.make()
        self.fail_times(app, config.BACKOFF_FROM)
        app.usage.reset_backoff()
        with fake_fetchers(claude=lambda: usage(current=7)):
            self.assertEqual(app.usage.fetch("claude").current.pct, 7)
        self.assertEqual((app.usage.failures, app.usage.retry_at), ({}, {}))

    def test_a_click_or_refresh_now_tries_at_once(self):
        app = self.make()
        self.fail_times(app, config.BACKOFF_FROM)
        app.select("claude")
        self.assertEqual(app.usage.retry_at, {})
        self.fail_times(app, config.BACKOFF_FROM)
        app.refresh_now()
        self.assertEqual(app.usage.retry_at, {})
        self.assertTrue(app.wake.is_set())
