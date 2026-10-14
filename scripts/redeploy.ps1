<#
Redeploy the tray app: stop the running one and start the new code through its scheduled task (the same way it starts at logon).

    powershell -File scripts\redeploy.ps1                 always redeploy
    powershell -File scripts\redeploy.ps1 -OnlyIfChanged  only when the code changed since the last successful deploy

Exit code 2 (with the reason on stderr) when the app didn't come up, which is what makes a Claude Code Stop hook hand the
problem back to Claude instead of leaving the screen without its app.
#>
param([switch]$OnlyIfChanged)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$stampFile = Join-Path $root ".claude\.deployed-hash"

# Claude Code sends the hook's context on stdin; a retry after a failed deploy must not block the session forever.
$again = $false
if ($OnlyIfChanged -and [Console]::IsInputRedirected) {  # (the hook's mode: a manual redeploy never waits on stdin)
    try {  # (not ReadToEnd: a caller that leaves its pipe open would hang us)
        $read = [Console]::In.ReadToEndAsync()
        if ($read.Wait(1500)) { $again = $read.Result -match '"stop_hook_active"\s*:\s*true' }
    } catch { }
}

function Get-CodeHash {
    $files = @(Get-ChildItem -Path (Join-Path $root "geekmagic") -Recurse -Filter *.py) +
             @(Get-Item (Join-Path $root "tray.py"))
    $text = ($files | Sort-Object FullName | ForEach-Object { "$($_.FullName)=$((Get-FileHash $_.FullName -Algorithm SHA1).Hash)" }) -join "`n"
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($text)
    ([System.BitConverter]::ToString([System.Security.Cryptography.SHA1]::Create().ComputeHash($bytes))) -replace "-", ""
}

function Get-Tray {
    Get-CimInstance Win32_Process -Filter "Name='pythonw.exe' or Name='python.exe'" |
        Where-Object { $_.CommandLine -like "*$root*tray.py*" }
}

$hash = Get-CodeHash
# (-OnlyIfChanged: an app that isn't running at all counts as out of date too)
if ($OnlyIfChanged -and (Get-Tray) -and (Test-Path $stampFile) -and ((Get-Content $stampFile -Raw).Trim() -eq $hash)) { exit 0 }

# The task has to be stopped as well as its process, or Task Scheduler still counts it as running and ignores the start.
Stop-ScheduledTask -TaskName GeekMagicUsage -ErrorAction SilentlyContinue
Get-Tray | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
for ($i = 0; $i -lt 20 -and (Get-Tray); $i++) { Start-Sleep -Milliseconds 500 }
Start-Sleep -Seconds 2
foreach ($attempt in 1..2) {
    Start-ScheduledTask -TaskName GeekMagicUsage
    Start-Sleep -Seconds 8
    if (Get-Tray) { break }
}

if (-not (Get-Tray)) {
    $log = if (Test-Path (Join-Path $root "tray.log")) { (Get-Content (Join-Path $root "tray.log") -Tail 15) -join "`n" } else { "(no log)" }
    [Console]::Error.WriteLine("Redeploy failed: the tray app is not running after the restart. Last log lines:`n$log")
    if ($again) { exit 0 }
    exit 2
}

Set-Content -Path $stampFile -Value $hash
Write-Output "Tray app redeployed with the current code."
