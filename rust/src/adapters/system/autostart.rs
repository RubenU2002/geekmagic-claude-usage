//! Start the tray app automatically at login: Task Scheduler (Windows), LaunchAgent (macOS), autostart (Linux).
//!
//! It uses the very same names as the Python version, so installing this one *replaces* that one's login entry (and the
//! other way round): only one of the two ever starts at login.
//!
//! ```text
//! geekmagic-usage --ip 192.168.1.18 --install-startup
//! geekmagic-usage --uninstall-startup
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::adapters::providers::executables::{home_dir, no_window};
use crate::adapters::system::terminal::list2cmdline;

/// Windows scheduled task.
pub const TASK_NAME: &str = "GeekMagicUsage";
/// macOS LaunchAgent label.
pub const LABEL: &str = "com.geekmagic.usage";
/// Linux autostart entry.
pub const DESKTOP_FILE: &str = "geekmagic-usage.desktop";
// What they were called when the app only showed Claude: installing replaces them, uninstalling removes them too.
const LEGACY_TASK_NAME: &str = "GeekMagicClaude";
const LEGACY_LABEL: &str = "com.geekmagic.claude-usage";
const LEGACY_DESKTOP_FILE: &str = "geekmagic-claude-usage.desktop";

/// launchd doesn't load your shell's PATH, so `claude` / `codex` in these folders would otherwise be missed.
fn mac_path() -> String {
    ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].iter().map(|s| s.to_string()).chain([home_dir().join(".local").join("bin").display().to_string()]).collect::<Vec<_>>().join(":")
}

fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn powershell(script: &str) -> Result<(), String> {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    let status = no_window(&mut cmd).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("powershell exited with {status}"))
    }
}

fn remove_task(name: &str) -> Result<(), String> {
    powershell(&format!(
        "Stop-ScheduledTask -TaskName {name} -ErrorAction SilentlyContinue\nUnregister-ScheduledTask -TaskName {name} -Confirm:$false -ErrorAction SilentlyContinue\n"
    ))
}

pub fn windows_task_script(command: &[String], workdir: &Path) -> String {
    format!(
        "$act = New-ScheduledTaskAction -Execute {} -Argument {} -WorkingDirectory {}\n\
         $trg = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME\n\
         $set = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable\n\
         Register-ScheduledTask -TaskName {TASK_NAME} -Action $act -Trigger $trg -Settings $set -Force | Out-Null\n\
         Start-ScheduledTask -TaskName {TASK_NAME}\n",
        ps_quote(&command[0]),
        ps_quote(&list2cmdline(&command[1..])),
        ps_quote(&workdir.display().to_string())
    )
}

fn install_windows(command: &[String], workdir: &Path) -> Result<String, String> {
    remove_task(LEGACY_TASK_NAME)?; // an old install under the old name would run a second copy
    powershell(&windows_task_script(command, workdir))?;
    Ok(format!("Scheduled task '{TASK_NAME}' created and started (runs at every logon)."))
}

fn uninstall_windows() -> Result<String, String> {
    remove_task(TASK_NAME)?;
    remove_task(LEGACY_TASK_NAME)?;
    Ok(format!("Scheduled task '{TASK_NAME}' removed."))
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn mac_plist_path(label: &str) -> PathBuf {
    home_dir().join("Library").join("LaunchAgents").join(format!("{label}.plist"))
}

pub fn build_mac_plist(command: &[String], workdir: &Path) -> String {
    let arguments: String = command.iter().map(|a| format!("    <string>{}</string>\n", xml_escape(a))).collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{LABEL}</string>\n  <key>ProgramArguments</key>\n  <array>\n{arguments}  </array>\n  <key>WorkingDirectory</key>\n  <string>{}</string>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n  <key>EnvironmentVariables</key>\n  <dict>\n    <key>PATH</key>\n    <string>{}</string>\n  </dict>\n</dict>\n</plist>\n",
        xml_escape(&workdir.display().to_string()),
        xml_escape(&mac_path())
    )
}

fn uid() -> String {
    Command::new("id").arg("-u").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|_| "501".into())
}

fn remove_mac(label: &str) -> PathBuf {
    let path = mac_plist_path(label);
    let _ = Command::new("launchctl").args(["bootout", &format!("gui/{}", uid()), &path.display().to_string()]).output();
    let _ = std::fs::remove_file(&path);
    path
}

fn install_mac(command: &[String], workdir: &Path) -> Result<String, String> {
    remove_mac(LEGACY_LABEL); // an old install under the old name would run a second copy
    let path = mac_plist_path(LABEL);
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    std::fs::write(&path, build_mac_plist(command, workdir)).map_err(|e| e.to_string())?;
    let domain = format!("gui/{}", uid());
    let _ = Command::new("launchctl").args(["bootout", &domain, &path.display().to_string()]).output(); // reload if already loaded
    let status = Command::new("launchctl").args(["bootstrap", &domain, &path.display().to_string()]).status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("launchctl bootstrap failed: {status}"));
    }
    Ok(format!("LaunchAgent {} loaded (runs at every login).", path.display()))
}

fn uninstall_mac() -> Result<String, String> {
    remove_mac(LEGACY_LABEL);
    Ok(format!("LaunchAgent {} removed.", remove_mac(LABEL).display()))
}

pub fn linux_desktop_path(name: &str) -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home_dir().join(".config"));
    config.join("autostart").join(name)
}

fn shell_join(command: &[String]) -> String {
    command
        .iter()
        .map(|a| if a.chars().all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c)) && !a.is_empty() { a.clone() } else { format!("'{}'", a.replace('\'', "'\"'\"'")) })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn build_desktop_entry(command: &[String], workdir: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=GeekMagic usage\nExec={}\nPath={}\nX-GNOME-Autostart-enabled=true\n",
        shell_join(command),
        workdir.display()
    )
}

fn install_linux(command: &[String], workdir: &Path) -> Result<String, String> {
    let _ = std::fs::remove_file(linux_desktop_path(LEGACY_DESKTOP_FILE)); // an old install under the old name would run twice
    let path = linux_desktop_path(DESKTOP_FILE);
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    std::fs::write(&path, build_desktop_entry(command, workdir)).map_err(|e| e.to_string())?;
    Ok(format!("Autostart entry {} created (runs at your next login).", path.display()))
}

fn uninstall_linux() -> Result<String, String> {
    let _ = std::fs::remove_file(linux_desktop_path(DESKTOP_FILE));
    let _ = std::fs::remove_file(linux_desktop_path(LEGACY_DESKTOP_FILE));
    Ok("Autostart entry removed.".into())
}

/// Start this program (`exe`) with `tray_args` at every login, from `workdir` (where its files live).
pub fn install(exe: &Path, tray_args: &[String], workdir: &Path) -> Result<String, String> {
    let mut command = vec![exe.display().to_string()];
    command.extend(tray_args.iter().cloned());
    match std::env::consts::OS {
        "windows" => install_windows(&command, workdir),
        "macos" => install_mac(&command, workdir),
        _ => install_linux(&command, workdir),
    }
}

pub fn uninstall() -> Result<String, String> {
    match std::env::consts::OS {
        "windows" => uninstall_windows(),
        "macos" => uninstall_mac(),
        _ => uninstall_linux(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd() -> Vec<String> {
        ["C:\\Program Files\\geek magic\\geekmagic-usage.exe", "--ip", "192.168.1.18", "--interval", "600"].map(String::from).to_vec()
    }

    #[test]
    fn the_windows_task_quotes_everything_and_replaces_the_pythons() {
        let script = windows_task_script(&cmd(), Path::new("C:\\Users\\me\\it's here"));
        assert!(script.contains("-Execute 'C:\\Program Files\\geek magic\\geekmagic-usage.exe'"), "{script}");
        assert!(script.contains("-Argument '--ip 192.168.1.18 --interval 600'"));
        assert!(script.contains("-WorkingDirectory 'C:\\Users\\me\\it''s here'"), "a quote in the folder is doubled");
        assert!(script.contains("Register-ScheduledTask -TaskName GeekMagicUsage"), "same name as the Python task");
        assert!(script.contains("-AtLogOn") && script.contains("-RestartCount 999"));
    }

    #[test]
    fn the_mac_plist_and_linux_entry_carry_the_command() {
        let plist = build_mac_plist(&["/opt/gm/geekmagic-usage".into(), "--ip".into(), "a&b".into()], Path::new("/opt/gm"));
        assert!(plist.contains("<string>com.geekmagic.usage</string>") && plist.contains("<string>a&amp;b</string>"));
        assert!(plist.contains("<key>SuccessfulExit</key>"), "restarted after a crash, not after Quit");
        let entry = build_desktop_entry(&["/opt/gm/geekmagic-usage".into(), "--night".into(), "22-7 x".into()], Path::new("/opt/gm"));
        assert!(entry.contains("Exec=/opt/gm/geekmagic-usage --night '22-7 x'") && entry.contains("Path=/opt/gm"), "{entry}");
    }
}
