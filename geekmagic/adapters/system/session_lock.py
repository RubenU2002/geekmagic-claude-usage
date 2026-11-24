"""Is this computer's screen locked? Best effort per OS; anything unknown or failing counts as "not locked"."""

from __future__ import annotations

import os
import subprocess
import sys

ERROR_ACCESS_DENIED = 5
DESKTOP_SWITCHDESKTOP = 0x0100


def _locked_windows() -> bool:
    """While the workstation is locked the input desktop is the secure one, which a normal process may neither
    open nor switch to."""
    import ctypes
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.OpenInputDesktop.restype = ctypes.c_void_p
    user32.SwitchDesktop.argtypes = [ctypes.c_void_p]
    user32.CloseDesktop.argtypes = [ctypes.c_void_p]
    desktop = user32.OpenInputDesktop(0, False, DESKTOP_SWITCHDESKTOP)
    if not desktop:
        return ctypes.get_last_error() == ERROR_ACCESS_DENIED  # any other failure: don't claim it's locked
    try:
        return not user32.SwitchDesktop(desktop)
    finally:
        user32.CloseDesktop(desktop)


def _locked_mac() -> bool:
    import Quartz  # pyobjc, which pystray already needs on macOS
    session = Quartz.CGSessionCopyCurrentDictionary()
    return bool(session and session.get("CGSSessionScreenIsLocked", 0))


def _locked_linux() -> bool:
    """systemd-logind's LockedHint (set by GNOME, KDE and most lockers)."""
    session = os.environ.get("XDG_SESSION_ID")
    if not session:
        return False
    out = subprocess.run(
        ["loginctl", "show-session", session, "-p", "LockedHint", "--value"],
        capture_output=True, text=True, timeout=2,
    ).stdout.strip()
    return out == "yes"


def is_locked() -> bool:
    try:
        if sys.platform == "win32":
            return _locked_windows()
        if sys.platform == "darwin":
            return _locked_mac()
        return _locked_linux()
    except Exception:
        return False


class SessionLockSensor:
    """The LockSensor port: is this computer's screen locked right now."""

    def is_locked(self) -> bool:
        return is_locked()
