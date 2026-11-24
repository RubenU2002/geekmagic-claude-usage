"""Where the app keeps its files (next to the launcher scripts, as it always has)."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TRAY_SCRIPT = ROOT / "tray.py"  # what the start-at-login entry runs
LOG_PATH = ROOT / "tray.log"
LOCK_PATH = ROOT / "tray.lock"  # held by the running app, see system.single_instance
STATE_PATH = ROOT / "tray_state.json"  # remembered between runs
