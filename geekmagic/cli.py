"""Push one provider's usage to the screen from the command line, once or on a loop (the tray app does much more).

    python geekmagic_usage.py --ip 192.168.1.18
    python geekmagic_usage.py --ip 192.168.1.18 --loop 60
    python geekmagic_usage.py --ip 192.168.1.18 --provider codex --animation bolt
"""

from __future__ import annotations

import argparse
import sys
import time

from geekmagic.adapters.device.client import GeekMagicDevice
from geekmagic.adapters.render.animations import ANIMATIONS
from geekmagic.adapters.render.renderer import PillowRenderer
from geekmagic.application import image_files
from geekmagic.bootstrap import build_registry
from geekmagic.core.errors import UsageError
from geekmagic.core.model import Usage
from geekmagic.core.ports import Renderer

PROVIDERS = build_registry()
_renderer = PillowRenderer()
_cleaned_up: set[str] = set()


def _cleanup_old_images(device: GeekMagicDevice) -> None:
    for name in image_files.LEGACY_NAMES:
        try:
            device.delete_image(name)
        except OSError:
            pass


def push_usage(device: GeekMagicDevice, provider: str, usage: Usage, animation: str, filename: str,
               renderer: Renderer = _renderer) -> None:
    """Render and upload `usage` as `filename`, and show it."""
    rendered = renderer.render(provider, usage, animation)
    device.upload(rendered.gif, filename)
    renderer.confirm(rendered)  # now it really is on the device
    device.show_image(filename)
    if device.ip not in _cleaned_up:  # one-time migration cleanup, not worth an extra request every push
        _cleanup_old_images(device)
        _cleaned_up.add(device.ip)
    print(
        f"[{usage.now:%H:%M:%S}] pushed [{rendered.animations[0]}] ({len(rendered.gif) / 1024:.1f} KB) — "
        f"current {usage.current.pct}% / weekly {usage.weekly.pct}%"
    )


def run_once(ip: str, animation: str, provider: str = "claude") -> None:
    push_usage(GeekMagicDevice(ip), provider, PROVIDERS[provider].fetch(), animation, image_files.image_name(provider))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ip", help="GeekMagic device IP, e.g. 192.168.1.18 (omit it to find the device on your network)")
    parser.add_argument("--discover", action="store_true", help="list the GeekMagic devices found on your network, then exit")
    parser.add_argument("--loop", type=int, metavar="SECONDS", help="repeat forever every N seconds")
    parser.add_argument(
        "--animation", default="auto", choices=[*ANIMATIONS, "auto", "random"],
        help="which animation: 'auto'/'random' (default) picks a random one from the provider's own set on every push, or name one",
    )
    parser.add_argument("--provider", default="claude", choices=list(PROVIDERS), help="which usage to show")
    args = parser.parse_args(argv)

    if args.discover or not args.ip:
        from geekmagic.adapters.device import discovery
        found = discovery.scan()
        if args.discover:
            print("\n".join(found) if found else "No GeekMagic device found on this network.")
            return 0 if found else 1
        if len(found) != 1:
            print("error: " + (f"several devices found ({', '.join(found)}); pick one with --ip"
                               if found else "no GeekMagic device found; pass --ip"), file=sys.stderr)
            return 1
        args.ip = found[0]
        print(f"found the device at {args.ip}")

    try:
        if args.loop:
            while True:
                try:
                    run_once(args.ip, args.animation, args.provider)
                except (UsageError, OSError, ValueError) as e:
                    print(f"warning: {e}", file=sys.stderr)
                time.sleep(args.loop)
        else:
            run_once(args.ip, args.animation, args.provider)
    except UsageError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0
