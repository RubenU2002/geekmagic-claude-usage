"""System-tray app: keeps a GeekMagic SmallTV updated and lets you flip between Claude and Codex usage.

Windows/Linux: left-click the tray icon to toggle, right-click for the menu.
macOS: clicking the menu-bar icon opens the menu; pick Claude or Codex there.

    python tray.py                                      # finds the screen on your network by itself
    python tray.py --ip 192.168.1.20                    # or tell it where the screen is
    python tray.py --install-startup                    # run it at every login
    python tray.py --uninstall-startup

Besides drawing the screen it sends a desktop notification when either provider's usage reaches 80 % / 95 %
(and when the limit resets), and it dims the screen if the numbers can't be refreshed for a few minutes. The
menu also has a split view and a stats view (both providers at once), pause (also while the computer is locked),
and brightness / night mode / dim-when-locked for the device's backlight.
"""

from __future__ import annotations

import argparse
import logging
import sys
from logging.handlers import RotatingFileHandler

from geekmagic import paths
from geekmagic.adapters.render.animations import ANIMATIONS
from geekmagic.adapters.device.client import BRIGHTNESS_RANGE
from geekmagic.adapters.providers import default_providers
from geekmagic.adapters.system import autostart, single_instance

_instance_lock = None  # kept for as long as the app runs: the lock goes with it


def setup_logging() -> None:
    """Log to tray.log (rotated). Done from main() so importing this module, e.g. from tests, writes nothing."""
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s",
        handlers=[RotatingFileHandler(paths.LOG_PATH, maxBytes=512 * 1024, backupCount=1, encoding="utf-8")],
    )


def night_hours(text: str) -> tuple[int, int]:
    """"22-7" -> (22, 7); argparse type for --night."""
    try:
        start, end = (int(part) for part in text.split("-"))
    except ValueError:
        raise argparse.ArgumentTypeError(f"expected START-END with hours 0-23, like 22-7 (got {text!r})") from None
    if not (0 <= start <= 23 and 0 <= end <= 23):
        raise argparse.ArgumentTypeError(f"hours must be 0-23 (got {text!r})")
    return start, end


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ip", help="GeekMagic device IP (omit it to find the screen on your network)")
    parser.add_argument("--interval", type=int, default=30, metavar="SECONDS")
    parser.add_argument(
        "--provider", choices=[p.key for p in default_providers()],
        help="which view to start on the very first run (afterwards the app comes back to the one you left)",
    )
    parser.add_argument("--animation", default="auto", choices=[*ANIMATIONS, "auto", "random"])
    parser.add_argument("--night", metavar="START-END", type=night_hours,
                        help="hours of the screen's night mode, 24-hour clock, e.g. 22-7 (default; turn it on from the menu)")
    parser.add_argument("--night-brightness", type=int, metavar="LEVEL", choices=range(BRIGHTNESS_RANGE[0], BRIGHTNESS_RANGE[1] + 1),
                        help=f"backlight level during the night mode ({BRIGHTNESS_RANGE[0]} to {BRIGHTNESS_RANGE[1]})")
    parser.add_argument("--install-startup", action="store_true", help="start this app automatically at login, then exit")
    parser.add_argument("--uninstall-startup", action="store_true", help="remove the automatic start, then exit")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> None:
    args = parse_args(argv)

    if args.uninstall_startup:
        print(autostart.uninstall())
        return
    if args.install_startup:
        print(autostart.install(paths.TRAY_SCRIPT, [
            *(["--ip", args.ip] if args.ip else []), "--interval", str(args.interval),
            *(["--provider", args.provider] if args.provider else []), "--animation", args.animation,
            *(["--night", f"{args.night[0]}-{args.night[1]}"] if args.night else []),
            *(["--night-brightness", str(args.night_brightness)] if args.night_brightness is not None else []),
        ]))
        return
    try:
        global _instance_lock
        _instance_lock = single_instance.acquire(paths.LOCK_PATH)
    except single_instance.AlreadyRunning:
        print("The tray app is already running; not starting a second copy.", file=sys.stderr)
        return
    setup_logging()
    if sys.platform == "darwin":
        import AppKit

        # pystray creates an NSApplication for the status item. Keep it out of the Dock.
        if not AppKit.NSApplication.sharedApplication().setActivationPolicy_(
            AppKit.NSApplicationActivationPolicyAccessory
        ):
            logging.getLogger("tray").warning("could not hide Python Dock icon")
    from geekmagic.bootstrap import run_tray
    run_tray(args.ip, args.interval, args.animation, args.provider, args.night, args.night_brightness)
