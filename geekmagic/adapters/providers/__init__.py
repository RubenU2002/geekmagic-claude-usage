"""The AI tools the app can show. Each module implements the Provider port (geekmagic.core.ports)."""

from __future__ import annotations

from geekmagic.adapters.providers.claude import ClaudeProvider
from geekmagic.adapters.providers.codex import CodexProvider
from geekmagic.core.ports import Provider


def default_providers() -> list[Provider]:
    """The providers the app ships with, in the order they cycle."""
    return [ClaudeProvider(), CodexProvider()]
