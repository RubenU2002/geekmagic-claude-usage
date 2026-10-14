"""Codex: its usage (from the account-limits call of its own app-server, which doesn't start a model turn)."""

from __future__ import annotations

import json
import queue
import subprocess
import sys
import threading
import time
from datetime import datetime
from pathlib import Path

from geekmagic.adapters.providers import codex_logs
from geekmagic.core import pace
from geekmagic.core.model import CodexExtra, Usage, Window
from geekmagic.core.ports import Provider
from geekmagic.core.errors import SignInNeeded, UsageError
from geekmagic.adapters.providers.executables import which


CODEX_TIMEOUT = 60  # (like Claude's read: slow connections need more than 30 s)


def find_codex() -> str | None:
    """`codex` from PATH, else the newest copy bundled with a VS Code-family ChatGPT/Codex extension."""
    found = which("codex")
    if found:
        return found
    binary = "codex.exe" if sys.platform == "win32" else "codex"
    bundled = [
        p
        for editor in (".vscode", ".vscode-insiders", ".cursor")
        for p in (Path.home() / editor / "extensions").glob(f"openai.chatgpt-*/bin/*/{binary}")
    ]
    bundled.sort(key=lambda p: p.stat().st_mtime, reverse=True)
    return str(bundled[0]) if bundled else None


def _fetch_codex_rate_limits() -> dict:
    """Query the authenticated Codex app-server without starting an AI turn."""
    executable = find_codex()
    if not executable:
        raise UsageError("`codex` CLI not found in PATH. Install Codex CLI and run `codex login`.")
    try:
        proc = subprocess.Popen(
            [executable, "app-server"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, encoding="utf-8", creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
    except OSError as e:
        raise UsageError("Could not start `codex app-server`.") from e

    responses = queue.Queue()

    def read_stdout() -> None:
        try:
            for line in proc.stdout:
                responses.put(line)
        finally:
            responses.put(None)

    reader = threading.Thread(target=read_stdout, daemon=True)
    reader.start()
    deadline = time.monotonic() + CODEX_TIMEOUT

    def send(message: dict) -> None:
        proc.stdin.write(json.dumps(message) + "\n")
        proc.stdin.flush()

    def receive(request_id: int) -> dict:
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise UsageError("Timed out querying Codex limits. Check your connection and `codex login`.")
            try:
                line = responses.get(timeout=remaining)
            except queue.Empty as e:
                raise UsageError("Timed out querying Codex limits. Check your connection and `codex login`.") from e
            if line is None:
                raise UsageError("Codex app-server exited before returning limits. Check `codex login`.")
            try:
                message = json.loads(line)
            except ValueError as e:
                raise UsageError("Codex app-server returned invalid JSON.") from e
            if not isinstance(message, dict) or message.get("id") != request_id:
                continue  # Notifications can arrive between request responses.
            if "error" in message:
                raise SignInNeeded("Codex could not read account limits. Not signed in? Run `codex login` with ChatGPT.")
            result = message.get("result")
            if not isinstance(result, dict):
                raise UsageError("Codex app-server returned an invalid response.")
            return result

    try:
        send({"id": 1, "method": "initialize", "params": {"clientInfo": {
            "name": "geekmagic_usage", "title": "GeekMagic Usage", "version": "1.0.0",
        }}})
        receive(1)
        send({"method": "initialized", "params": {}})
        send({"id": 2, "method": "account/rateLimits/read"})
        result = receive(2)
        buckets = result.get("rateLimitsByLimitId")
        # Prefer the Codex bucket when multiple model quotas are returned.
        limits = buckets.get("codex") if isinstance(buckets, dict) else None
        if limits is None:
            limits = result.get("rateLimits")
        if not isinstance(limits, dict) or not any(limits.get(w) for w in ("primary", "secondary")):
            raise SignInNeeded("Codex returned no account limits. Sign in with ChatGPT using `codex login`.")
        return {**limits, "_reset_credits": result.get("rateLimitResetCredits")}  # the free resets sit beside the limits
    except OSError as e:
        raise UsageError("Lost connection to Codex app-server.") from e
    finally:
        try:
            proc.stdin.close()
        except OSError:
            pass
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        reader.join(timeout=2)
        proc.stdout.close()


def fetch_codex_usage() -> Usage:
    """Fetch fresh account limits, even when the user is not actively using Codex."""
    limits = _fetch_codex_rate_limits()
    now = datetime.now().astimezone()

    def window(block: dict | None) -> Window:
        if block is None:
            return Window()
        if not isinstance(block, dict):
            raise UsageError("Codex returned an invalid quota window.")
        try:
            reset = datetime.fromtimestamp(block["resetsAt"]).astimezone() if block.get("resetsAt") else None
            pct = float(block["usedPercent"])
        except (KeyError, TypeError, ValueError, OverflowError, OSError) as e:
            raise UsageError("Codex returned an invalid quota window.") from e
        # Codex says how long each window really is (300 and 10080 minutes at the time of writing).
        minutes = block.get("windowDurationMins")
        return Window(pct, reset, minutes if isinstance(minutes, (int, float)) else None)

    usage = Usage(title="Codex", current=window(limits.get("primary")), weekly=window(limits.get("secondary")), now=now,
                  codex_extra=_codex_extra(limits))
    return pace.annotate(usage)


def _codex_extra(limits: dict) -> CodexExtra:
    """The free rate-limit resets Codex has granted you, for its own screen: how many are left and in how many days
    the next one expires. Every field is optional in Codex's reply."""
    resets = limits.get("_reset_credits") if isinstance(limits.get("_reset_credits"), dict) else {}
    credits = resets.get("credits")  # (Codex sometimes sends null instead of leaving it out)
    available = [c for c in (credits if isinstance(credits, list) else []) if isinstance(c, dict) and c.get("status") == "available"]
    expiries = [c["expiresAt"] for c in available if isinstance(c.get("expiresAt"), (int, float))]
    return CodexExtra(
        free_resets=resets.get("availableCount", len(available)) or 0,
        next_reset_expires_days=max(0, round((min(expiries) - time.time()) / 86400)) if expiries else None,
    )


class CodexProvider(Provider):
    key = "codex"
    title = "Codex"
    login_args = ("login",)
    install_hint = "Install the Codex CLI, then run `codex login`."

    def fetch(self) -> Usage:
        return fetch_codex_usage()

    def find_cli(self) -> str | None:
        return find_codex()

    def local_stats(self, now_epoch: float) -> dict | None:
        return codex_logs.stats(now_epoch)

    def agent_state(self, now_epoch: float) -> dict:
        return codex_logs.agent_state(now_epoch)
