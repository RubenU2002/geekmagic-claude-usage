#!/usr/bin/env python3
"""System-tray app for the GeekMagic SmallTV: Claude and Codex usage, a menu, notifications.

    python tray.py                                      # finds the screen on your network by itself
    python tray.py --ip 192.168.1.20                    # or tell it where the screen is
    python tray.py --install-startup                    # run it at every login

The code lives in the `geekmagic` package (see geekmagic/app/); this is just the launcher the start-at-login entry runs.
"""

from geekmagic.tray_main import main

if __name__ == "__main__":
    main()
