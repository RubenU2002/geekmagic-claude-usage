"""Open a terminal window that runs a command and stays open: how a provider's sign-in is started.

Each provider's own CLI does the sign-in (a browser opens and you approve it there); this only starts it, in a visible
terminal, since the CLIs ask questions. What to run is up to the caller (see geekmagic.app.signin).
"""

from __future__ import annotations

import os
import shlex
import shutil
import subprocess
import sys


LINUX_TERMINALS = (
    ("x-terminal-emulator", ["-e"]), ("gnome-terminal", ["--"]), ("konsole", ["-e"]),
    ("xfce4-terminal", ["-x"]), ("kitty", []), ("alacritty", ["-e"]), ("xterm", ["-e"]),
)


def terminal_command(argv: list[str], title: str, platform: str | None = None) -> list[str] | str | None:
    """What to run to open a terminal window that runs `argv` and stays open, or None if no terminal can be found.
    (Windows gets one string: cmd.exe wants the command line exactly as written.)"""
    platform = platform or sys.platform
    if platform == "win32":
        # `cmd /k ""C:\\a b\\x.exe" arg"`: cmd drops the outer pair of quotes, so a path with spaces survives
        return f'cmd.exe /c start "{title}" cmd.exe /k "{subprocess.list2cmdline(argv)}"'
    if platform == "darwin":
        script = " ".join(shlex.quote(a) for a in argv).replace("\\", "\\\\").replace('"', '\\"')
        return ["osascript", "-e", 'tell application "Terminal" to activate',
                "-e", f'tell application "Terminal" to do script "{script}"']
    for terminal, flags in LINUX_TERMINALS:
        if shutil.which(terminal):
            return [terminal, *flags, *argv]
    return None


def launch(argv: list[str], title: str) -> str:
    """Run `argv` in a new terminal window that stays open (a provider's sign-in): "started", or "failed" (no terminal
    to open it in). `GEEKMAGIC_NO_LOGIN=1` turns it off (the tests use it: no test may open a window)."""
    if os.environ.get("GEEKMAGIC_NO_LOGIN"):
        return "failed"
    command = terminal_command(argv, title)
    if not command:
        return "failed"
    try:
        subprocess.Popen(command, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                         env={**os.environ})
    except OSError:
        return "failed"
    return "started"


class Terminal:
    """The TerminalLauncher port."""

    def launch(self, argv: list[str], title: str) -> str:
        return launch(argv, title)
