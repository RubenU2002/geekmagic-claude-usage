"""What is on the screen: one provider's, or one of the views of two providers at once (the "mode")."""

from __future__ import annotations

from collections.abc import Collection

from geekmagic.core.views import PANEL_KEYS


class ViewState:
    def __init__(self, provider_keys: Collection[str], first_run_provider: str | None = None) -> None:
        self.provider_keys = tuple(provider_keys)
        self.first_run_provider = first_run_provider  # --provider: only the very first run's choice
        self.mode: str | None = None  # a view of two providers instead of one (only the menu turns these on), or None
        self.provider = "claude"  # the last single-provider view, also what the left-click goes on from

    @property
    def key(self) -> str:
        """What the screen should be showing: a provider, or a view of two."""
        return self.mode or self.provider

    # The flag the split view was built on, kept as a property so `view.split` reads and writes naturally.
    @property
    def split(self) -> bool:
        return self.mode == "split"

    @split.setter
    def split(self, value: bool) -> None:
        self.mode = "split" if value else (None if self.mode == "split" else self.mode)

    def restore(self, saved: dict) -> None:
        """The view comes back exactly as it was left; --provider is only the very first run's choice."""
        first_run_provider = self.first_run_provider
        saved_view = saved.get("view") if saved.get("view") in (*self.provider_keys, *PANEL_KEYS) else None
        saved_provider = saved.get("provider") if saved.get("provider") in self.provider_keys else None
        view = saved_view or first_run_provider or "claude"
        self.mode = view if view in PANEL_KEYS else None
        self.provider = (saved_provider or first_run_provider or "claude") if self.mode else view

    def snapshot(self) -> dict:
        return {"view": self.key, "provider": self.provider}
