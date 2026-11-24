"""Desktop notifications, and the two switches that turn them off."""

from __future__ import annotations

import logging

from geekmagic.core.ports import Notifier

log = logging.getLogger("tray")


class Notifications:
    def __init__(self, backup: Notifier, primary: Notifier | None = None) -> None:
        self.primary = primary  # the tray app's own notification: the one shown (as "Python")
        self.backup = backup  # the system's own route, only if the primary couldn't (sending both shows every alert twice)
        self.enabled = True  # every notification
        self.agent_events = True  # the ones about an agent finishing or waiting for you

    def send(self, title: str, message: str) -> None:
        log.info("notification: %s - %s", title, message)
        if not self.enabled:
            return
        try:
            if self.primary is not None:
                try:
                    self.primary.send(title, message)
                    return
                except Exception:
                    log.warning("the tray's own notification failed; trying the system's", exc_info=True)
            self.backup.send(title, message)
        except Exception:  # it must never break the update loop
            log.warning("could not show the notification", exc_info=True)

    def restore(self, saved: dict) -> None:
        self.enabled = bool(saved.get("notify", True))
        self.agent_events = bool(saved.get("notify_done", True))

    def snapshot(self) -> dict:
        return {"notify": self.enabled, "notify_done": self.agent_events}
