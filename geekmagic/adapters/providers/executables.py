"""Finding a command-line tool, including where a login task's short PATH wouldn't look."""

from __future__ import annotations

import os
import shutil
from pathlib import Path


def which(name: str) -> str | None:
    """Like shutil.which, plus the usual install folders a background job (launchd, Task Scheduler) may not have on PATH."""
    extra = [str(Path.home() / ".local" / "bin"), "/opt/homebrew/bin", "/usr/local/bin"]
    return shutil.which(name) or shutil.which(name, path=os.pathsep.join(extra))
