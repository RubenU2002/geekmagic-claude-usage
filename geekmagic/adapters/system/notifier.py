"""Desktop notifications through each operating system's own mechanism.

The tray library's balloon (`pystray`'s `icon.notify`) turned out not to show up on a stock Windows 11 setup, where
no notification source was ever registered for `pythonw`. The native routes do:

* Windows: a toast through PowerShell's own app identity (the same route PowerShell scripts and most tools use);
* macOS: `osascript`'s `display notification`;
* Linux: `notify-send`.

Title and message go to the helper through environment variables, never spliced into a command, so quotes, newlines
or backticks in them can't break (or inject into) anything. Sending doesn't wait for the helper: starting PowerShell
takes most of a second and the callers are loops that have other things to do.
"""

from __future__ import annotations

import os
import subprocess
import sys
import threading

# Windows' toast API is reached through PowerShell. {1AC14E77-...}\WindowsPowerShell\... is PowerShell's own
# AppUserModelID, which Windows already knows how to show notifications for.
_WINDOWS_APP_ID = "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe"
_WINDOWS_SCRIPT = (
    "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null;"
    "$xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02);"
    "$text = $xml.GetElementsByTagName('text');"
    "$text.Item(0).AppendChild($xml.CreateTextNode($env:GM_TITLE)) | Out-Null;"
    "$text.Item(1).AppendChild($xml.CreateTextNode($env:GM_MESSAGE)) | Out-Null;"
    "$toast = [Windows.UI.Notifications.ToastNotification]::new($xml);"
    f"[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{_WINDOWS_APP_ID}').Show($toast)"
)
_MAC_SCRIPT = 'display notification (system attribute "GM_MESSAGE") with title (system attribute "GM_TITLE")'
HELPER_TIMEOUT = 20  # seconds before a helper that hung is killed


def command(title: str, message: str, platform: str | None = None) -> tuple[list[str], dict[str, str]] | None:
    """(argv, extra environment) that shows a notification on `platform` (default: this one), or None if there's no
    native route for it."""
    platform = platform or sys.platform
    env = {"GM_TITLE": title, "GM_MESSAGE": message}
    if platform == "win32":
        return ["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", _WINDOWS_SCRIPT], env
    if platform == "darwin":
        return ["osascript", "-e", _MAC_SCRIPT], env
    if platform.startswith("linux"):
        return ["notify-send", "--app-name=GeekMagic usage", title, message], env
    return None


def _reap(proc: subprocess.Popen) -> None:
    try:
        proc.wait(HELPER_TIMEOUT)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()


def send(title: str, message: str) -> bool:
    """Show a notification natively. True if the helper was started (the OS may still decide not to show it, e.g. in
    do-not-disturb); False if there's no native route or the helper isn't installed, so the caller can fall back."""
    if os.environ.get("GEEKMAGIC_NO_NOTIFY"):  # a kill switch (the tests set it, so they never pop up real notifications)
        return True
    built = command(title, message)
    if built is None:
        return False
    argv, extra_env = built
    try:
        proc = subprocess.Popen(
            argv, env={**os.environ, **extra_env}, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
    except OSError:  # e.g. notify-send isn't installed
        return False
    threading.Thread(target=_reap, args=(proc,), daemon=True).start()
    return True


class SystemNotifier:
    """The Notifier port, through the operating system's own mechanism."""

    def send(self, title: str, message: str) -> None:
        if not send(title, message):
            raise OSError("this system has no way to show a notification")
