# geekmagic-usage (Rust)

The Rust version of the tray app, written next to the Python one (which is untouched and still the one the `GeekMagicUsage`
scheduled task runs). Same screens, same menu, same device protocol, same architecture (core / application / adapters, see
`../ARCHITECTURE.md`); two differences on purpose:

* **Low memory.** No interpreter, no image library, no tray library with its own runtime: one small native process.
* **Webhooks instead of a timer.** The screen is refreshed when something *happens*, not every 30 s.

## Build and run

```powershell
cd rust
cargo build --release                  # target\release\geekmagic-usage.exe
target\release\geekmagic-usage.exe     # the tray app; finds the screen by itself, or --ip 192.168.1.18
target\release\geekmagic-usage.exe --help
```

Windows needs the MSVC build tools **and the Windows SDK** (Visual Studio Installer > "Windows 11 SDK"). The toolchain is
pinned by `rust-version = "1.84"`; `.cargo/config.toml` makes cargo pick dependency versions that build on it.

It shares `tray.lock` and `tray_state.json` with the Python app, so the two **cannot run at once** (the second one exits) and
either picks up the view, brightness and remembered address the other left. Its log is `tray-rs.log`.

### Switching over

```powershell
Stop-ScheduledTask GeekMagicUsage                        # stop the Python app
target\release\geekmagic-usage.exe --install-startup     # replaces the login task with this one (same name) and starts it
```

(The Python Stop hook in `.claude/settings.json` only watches `geekmagic/` and `tray.py`, so working on `rust/` never redeploys
anything.)

Going back: stop it and run `python tray.py --install-startup`. `--uninstall-startup` removes the login entry either way.

## How the screen is refreshed

A cycle (read the usage, draw, upload) starts on an **event**:

| Event | Comes from |
|---|---|
| a webhook | `POST /refresh`, `POST /hook/<provider>/<event>` on `127.0.0.1:8765` |
| a click or menu choice | the tray icon |
| an agent started, stopped or is waiting | the providers' local logs, looked at when a webhook arrives and every 2 s while one works |
| the screen came back / the PC unlocked | the app itself |

There is **no 30 s timer**. The only timer is a slow fallback (`--interval`, default 900 s; `--interval 0` removes it), so the
screen still gets refreshed now and then if no webhook ever arrives. Without a refresh the "Resets in ..." countdown and the
"Updated" time keep their last values: that is the price of not polling.

Many webhooks in a row make one read: they only wake the worker, which does what a click does.

### No screen: the tray is the screen

While the GeekMagic can't be reached the app keeps reading the usage and still draws every picture it would have uploaded
(only again when its data changed, so the 10 s retries don't redraw the same thing). **Left-clicking the icon then opens a
window that plays it**: the same 240 x 240 GIF at the real screen's size. Click the icon again to close it; a click *inside* the
window goes on to the next provider. **Pantalla simulada** in the menu opens it even when the real screen is there. With the
screen reachable a click on the icon switches provider, as before. (Windows only; `--no-tray` has no window.)

### Webhook

```text
POST http://127.0.0.1:8765/refresh                    read every provider again (?provider=claude for one)
POST http://127.0.0.1:8765/hook/claude/Stop           a tool says something happened
POST http://127.0.0.1:8765/hook/claude                the event is in the body ({"hook_event_name": "Stop"}) or ?event=
GET  http://127.0.0.1:8765/health
```

`UserPromptSubmit`, `SessionStart`, `PreToolUse`, `Notification` and `PermissionRequest` only make the app look at the agents
(the "Working" / "Waiting" tag); any other event (`Stop`, `SessionEnd`, ...) also reads that provider's usage again.

It listens on loopback only, refuses any request whose `Host` isn't the loopback address (so a web page can't reach it through
DNS rebinding) and, with `--webhook-token` / `GEEKMAGIC_WEBHOOK_TOKEN`, any request without that token in `X-GeekMagic-Token`.
`--no-webhook` turns it off; `--webhook-port` moves it.

### Claude Code hooks

```powershell
geekmagic-usage.exe --print-hooks
```

prints a `hooks` block for `~/.claude/settings.json` (it does not edit it). Each hook runs
`geekmagic-usage.exe notify claude <Event>`, which POSTs to the running app and **always exits 0**, so it can't get in the way of
Claude Code, and needs neither `curl` nor a particular shell. For Codex, anything that can send an HTTP POST can call
`/refresh` or `/hook/codex/<event>`.

## What was checked

Verified in this repo (`cargo test`: 158 tests, all passing):

* every rule of `core` (pace, alerts, activity, sessions) with the Python tests' expectations;
* the drawing: every animation and every view renders; GIFs decode with Pillow, have the same structure as the Python ones
  (240x240, 130 ms, disposal 1, loop forever, later frames only the changed rectangle) and are smaller; stale dimming
  matches Pillow's `Image.blend` to the digit;
* the device client against a fake SmallTV: upload, list, download, theme selection, brightness / night mode, **cancelling an
  upload in flight**, an unreachable device;
* the providers: Claude `/usage` text, reset-date parsing, Codex JSON, both log formats, "working / waiting" detection;
* the whole application on fakes, **including an end-to-end run with no timer** (nothing happens by itself; a webhook makes
  exactly one more read and upload);
* the webhook: routes, token, Host check, oversized bodies, a stalled client.

Run for real on this machine: reading `claude /usage` and `codex app-server` (same numbers and reset times as the Python
reader), the tray icon (a simulated left click through the icon's window switched the view), the webhook, the state file
(same keys as Python's), and memory:

| | working set | private bytes |
|---|---|---|
| Python app (long-running, as found) | 28.2 MB | 40.3 MB |
| Rust, single-provider view, screen unreachable | 16.6 MB | 4.0 MB |
| Rust, stats view (counts a month of logs), screen unreachable | 21.0 MB (peak 21.6) | 13.3 MB |

The loads are not identical (the Python one has its device and views warmed up); treat it as an order of magnitude, not a benchmark.

## Not verified

* **The real GeekMagic hardware.** The Rust client has only talked to a fake device, and the device was never sent a Rust-made
  GIF (the Python app owns it). Pillow reads those GIFs fine, but whether the SmallTV's own decoder does is untested. Try it with
  `geekmagic-usage push --ip <device> --provider claude` before relying on it.
* **Claude Code's hook event names and settings shape.** Only the `Stop` event and the `{"type": "command", "command", "timeout"}`
  shape are confirmed by this repo (`../.claude/settings.json`). `UserPromptSubmit`, `Notification` and `SessionEnd` come from
  memory: after pasting the hooks, check `tray-rs.log` for `webhook: claude UserPromptSubmit` lines.
* Codex has no hook wired up; its agent state still comes from its logs.
* macOS and Linux: everything except the tray compiles there, but the tray icon, the lock sensor on macOS and the start-at-login
  entries were not run. Use `--no-tray` (webhooks only) off Windows.
* Pixel-for-pixel equality with the Python screens: text is drawn by `ab_glyph` instead of FreeType, so glyph edges differ slightly.

## Layout

```text
src/core/         model, rules, ports (traits)            -- knows nothing of any adapter
src/application/  use cases: scheduler, screen, agents, ... -- written against the ports; tests on fakes (testing.rs)
src/adapters/     providers, device, render, store, system, tray, webhook
src/bootstrap.rs  composition root, files, logging
src/cli.rs        command line (tray, push, notify, --print-hooks, --install-startup)
assets/           Aileron Regular (Pillow's default font, CC0)
```
