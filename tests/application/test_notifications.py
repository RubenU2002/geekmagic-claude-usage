"""Usage alerts and desktop notifications: what fires, when, and what never gets in the way of the update loop."""
from tests.support import AppTestCase, usage


class NotificationTests(AppTestCase):
    def check(self, app, provider, reading):
        """What a successful read does with the alerts (the tracker no longer saves; the app's persistence does)."""
        app.usage.alert_tracker.check(provider, reading)
        app.persistence.save()

    # --- alerts ---------------------------------------------------------------------------------------

    def test_notifies_when_thresholds_are_crossed_and_on_reset(self):
        app = self.make()
        for pct in (50, 82, 83, 96, 97, 3):
            self.check(app, "claude", usage(current=pct))
        messages = [m for _, m in self.notes]
        self.assertEqual(len(messages), 3, messages)
        self.assertIn("80 %", messages[0])
        self.assertIn("95 %", messages[1])
        self.assertIn("reinició", messages[2])
        self.assertTrue(all(title == "Claude" for title, _ in self.notes))

    def test_weekly_and_the_other_provider_are_tracked_separately(self):
        app = self.make()
        self.check(app, "claude", usage(current=10, weekly=85))
        self.check(app, "codex", usage(current=82, weekly=5, title="Codex"))
        self.assertEqual([t for t, _ in self.notes], ["Claude", "Codex"])
        self.assertIn("semana", self.notes[0][1])
        self.assertIn("sesión", self.notes[1][1])

    def test_alerts_that_already_fired_dont_fire_again_after_a_restart(self):
        first = self.make()
        self.check(first, "claude", usage(current=85))
        self.assertEqual(len(self.notes), 1)
        second = self.make()  # the app restarts: state comes back from disk
        self.check(second, "claude", usage(current=86))
        self.assertEqual(len(self.notes), 1)

    def test_notifications_can_be_switched_off_but_state_keeps_tracking(self):
        app = self.make()
        app.toggle_notifications()
        self.assertFalse(app.notifications.enabled)
        self.check(app, "claude", usage(current=96))
        self.assertEqual(self.notes, [])
        self.assertEqual(app.usage.alert_tracker.state["claude/current"]["level"], 2)
        self.assertFalse(self.make().notifications.enabled)  # the choice is remembered too

    def test_a_failing_notification_backend_never_breaks_the_loop(self):
        app = self.make()
        app.ui.fail_notifications = True

        def broken(title, message):
            raise OSError("helper exploded")

        self.check(app, "claude", usage(current=96))  # must not raise: the system's route steps in
        self.assertEqual(len(self.backups), 1)
        self.fake_notifier.send = broken  # the system's route fails too
        self.check(app, "codex", usage(current=96, title="Codex"))  # nor must this
        self.assertEqual((self.notes, len(self.backups)), ([], 1))

    def test_each_alert_is_shown_once_by_the_tray_and_the_system_route_is_only_a_backup(self):
        app = self.make()
        app.notifications.send("Claude", "hola")
        self.assertEqual((self.notes, self.backups), ([("Claude", "hola")], []), "one notification, never two")
        app.ui.fail_notifications = True
        app.notifications.send("Claude", "otra vez")
        self.assertEqual(self.backups, [("Claude", "otra vez")], "the tray couldn't: the system's own route steps in")

    def test_turned_off_notifications_send_nothing_at_all(self):
        app = self.make()
        app.toggle_notifications()
        app.notifications.send("Claude", "hola")
        self.assertEqual((self.notes, self.backups), ([], []))
