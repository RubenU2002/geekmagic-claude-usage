"""Identifies the code that draws the screens.

The device keeps the images it was sent, so after an update (new labels, colours, animations...) what's stored is out of
date; a changed id makes the app re-upload every view once.
"""

import hashlib
from pathlib import Path

_PACKAGE = Path(__file__).resolve().parents[2]  # geekmagic/


def compute() -> str:
    """A hash of everything that decides how a screen looks: the drawing code and the pure logic it draws from."""
    sources = sorted([*(_PACKAGE / "adapters" / "render").rglob("*.py"), *(_PACKAGE / "core").glob("*.py")])
    return hashlib.sha1(b"".join(path.read_bytes() for path in sources)).hexdigest()[:12]


RENDER_ID = compute()
