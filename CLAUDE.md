# GeekMagic usage tray app

## Redeploy on every code change

The tray app runs from a scheduled task (`GeekMagicUsage`) on this machine, so a change to the code does nothing until the app is restarted.
After changing code under `geekmagic/` or `tray.py`: run the tests, then redeploy and look at the log.

    powershell -File scripts\redeploy.ps1
    Get-Content tray.log -Tail 20

A Stop hook (`.claude/settings.json`) does the redeploy at the end of every turn that changed the code (`-OnlyIfChanged`), and sends the
failure back if the app didn't come up. Don't leave a turn with code that isn't running.
