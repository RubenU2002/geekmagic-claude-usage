//! Desktop notifications through each operating system's own mechanism.
//!
//! * Windows: a toast through PowerShell's own app identity (the same route PowerShell scripts and most tools use);
//! * macOS: `osascript`'s `display notification`;
//! * Linux: `notify-send`.
//!
//! Title and message go to the helper through environment variables, never spliced into a command, so quotes, newlines
//! or backticks in them can't break (or inject into) anything. Sending doesn't wait for the helper: starting PowerShell
//! takes most of a second and the callers are loops that have other things to do.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::adapters::providers::executables::no_window;
use crate::core::ports::Notifier;

/// {1AC14E77-...}\WindowsPowerShell\... is PowerShell's own AppUserModelID, which Windows already knows how to show
/// notifications for.
const WINDOWS_APP_ID: &str = "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";
const MAC_SCRIPT: &str = "display notification (system attribute \"GM_MESSAGE\") with title (system attribute \"GM_TITLE\")";
/// Seconds before a helper that hung is killed.
const HELPER_TIMEOUT: Duration = Duration::from_secs(20);

fn windows_script() -> String {
    format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null;\
         $xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02);\
         $text = $xml.GetElementsByTagName('text');\
         $text.Item(0).AppendChild($xml.CreateTextNode($env:GM_TITLE)) | Out-Null;\
         $text.Item(1).AppendChild($xml.CreateTextNode($env:GM_MESSAGE)) | Out-Null;\
         $toast = [Windows.UI.Notifications.ToastNotification]::new($xml);\
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{WINDOWS_APP_ID}').Show($toast)"
    )
}

/// (program, arguments) that shows a notification on `platform`, or None if there's no native route for it. The title and
/// message travel as the `GM_TITLE` / `GM_MESSAGE` environment variables (and as arguments where the tool wants them).
pub fn command(title: &str, message: &str, platform: &str) -> Option<(String, Vec<String>)> {
    let s = String::from;
    match platform {
        "windows" => Some((s("powershell"), vec![s("-NoProfile"), s("-NonInteractive"), s("-ExecutionPolicy"), s("Bypass"), s("-Command"), windows_script()])),
        "macos" => Some((s("osascript"), vec![s("-e"), s(MAC_SCRIPT)])),
        "linux" => Some((s("notify-send"), vec![s("--app-name=GeekMagic usage"), title.to_string(), message.to_string()])),
        _ => None,
    }
}

/// Show a notification natively. True if the helper was started (the OS may still decide not to show it, e.g. in
/// do-not-disturb); false if there's no native route or the helper isn't installed, so the caller can fall back.
pub fn send(title: &str, message: &str) -> bool {
    if std::env::var_os("GEEKMAGIC_NO_NOTIFY").is_some() {
        return true; // a kill switch (the tests set it, so they never pop up real notifications)
    }
    let Some((program, args)) = command(title, message, std::env::consts::OS) else { return false };
    let mut cmd = Command::new(program);
    cmd.args(args).env("GM_TITLE", title).env("GM_MESSAGE", message).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let Ok(mut child) = no_window(&mut cmd).spawn() else { return false };
    let _ = std::thread::Builder::new().stack_size(128 * 1024).spawn(move || {
        let deadline = Instant::now() + HELPER_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    });
    true
}

/// The Notifier port, through the operating system's own mechanism.
pub struct SystemNotifier;

impl Notifier for SystemNotifier {
    fn send(&self, title: &str, message: &str) -> Result<(), String> {
        if send(title, message) {
            Ok(())
        } else {
            Err("this system has no way to show a notification".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_platform_has_its_own_route_and_keeps_text_out_of_the_command() {
        let (prog, args) = command("T'itle", "a \"quoted\" `message`\nwith lines", "windows").unwrap();
        assert_eq!(prog, "powershell");
        let script = args.last().unwrap();
        assert!(script.contains("$env:GM_TITLE") && script.contains("$env:GM_MESSAGE"));
        assert!(!script.contains("quoted"), "the message never enters the script");
        let (prog, args) = command("T", "m", "macos").unwrap();
        assert_eq!((prog.as_str(), args[0].as_str()), ("osascript", "-e"));
        assert_eq!(args[1], MAC_SCRIPT, "the macOS script reads the text from the environment too");
        let (prog, args) = command("T", "m", "linux").unwrap();
        assert_eq!((prog.as_str(), &args[1..]), ("notify-send", &["T".to_string(), "m".to_string()][..]));
        assert!(command("T", "m", "plan9").is_none());
    }
}
