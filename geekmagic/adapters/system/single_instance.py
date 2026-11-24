"""Only one tray app at a time.

Two copies would fight over the screen (each uploading its own images), read the providers twice and send every
notification twice. They happen easily: the login task starting while you launch it by hand, or a restart that begins
before the old process has gone. The first one to start holds a lock on a file for as long as it lives; a second
finds it held and exits. The operating system drops the lock when the process ends, however it ends, so there's
nothing to clean up after a crash.
"""

from __future__ import annotations

import sys
from pathlib import Path


class AlreadyRunning(Exception):
    pass


def acquire(path: Path):
    """Take the lock on `path` and return the open file: keep a reference to it for as long as the app runs.
    Raises AlreadyRunning if another process holds it."""
    handle = open(path, "a+b")
    try:
        if sys.platform == "win32":
            import msvcrt
            handle.seek(0)
            if not handle.read(1):
                handle.write(b"\0")  # a byte to lock (Windows locks byte ranges)
                handle.flush()
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        handle.close()
        raise AlreadyRunning(f"another instance holds {path}") from None
    return handle


def release(handle) -> None:
    """Let go early (the tests do; the app just exits)."""
    try:
        if sys.platform == "win32":
            import msvcrt
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            import fcntl
            fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
    finally:
        handle.close()
