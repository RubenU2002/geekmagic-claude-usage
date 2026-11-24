"""The providers in use, in the order they cycle."""

from __future__ import annotations

from collections.abc import Iterable, Iterator

from geekmagic.core.ports import Provider


class ProviderRegistry:
    def __init__(self, providers: Iterable[Provider]) -> None:
        self._by_key = {p.key: p for p in providers}
        self.titles: dict[str, str] = {key: p.title for key, p in self._by_key.items()}  # key -> display name
        self._key_of = {p.title: key for key, p in self._by_key.items()}  # display name -> key

    def __getitem__(self, key: str) -> Provider:
        return self._by_key[key]

    def __iter__(self) -> Iterator[str]:
        return iter(self._by_key)

    def __contains__(self, key: object) -> bool:
        return key in self._by_key

    def __len__(self) -> int:
        return len(self._by_key)

    def keys(self):
        return self._by_key.keys()

    def values(self):
        return self._by_key.values()

    def items(self):
        return self._by_key.items()

    def key_of(self, title: str | None) -> str | None:
        """A reading says which provider it is by title: its key."""
        return self._key_of.get(title)
