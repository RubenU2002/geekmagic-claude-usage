//! Open a terminal window that runs a command and stays open: how a provider's sign-in is started.
//!
//! Each provider's own CLI does the sign-in (a browser opens and you approve it there); this only starts it, in a visible
//! terminal, since the CLIs ask questions. What to run is up to the caller (see `application::signin`).

use std::process::{Command, Stdio};

use crate::adapters::providers::executables::{no_window, which};
use crate::core::ports::TerminalLauncher;

const LINUX_TERMINALS: [(&str, &[&str]); 7] = [
    ("x-terminal-emulator", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("xterm", &["-e"]),
];

/// Quote arguments into one command line the way Windows programs read it (Python's `subprocess.list2cmdline`).
pub fn list2cmdline(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            let needs_quotes = arg.is_empty() || arg.contains([' ', '\t']);
            let mut out = String::new();
            let mut backslashes = 0;
            if needs_quotes {
                out.push('"');
            }
            for c in arg.chars() {
                match c {
                    '\\' => backslashes += 1,
                    '"' => {
                        out.push_str(&"\\".repeat(backslashes * 2 + 1));
                        out.push('"');
                        backslashes = 0;
                    }
                    _ => {
                        out.push_str(&"\\".repeat(backslashes));
                        backslashes = 0;
                        out.push(c);
                    }
                }
            }
            out.push_str(&"\\".repeat(backslashes));
            if needs_quotes {
                out.push_str(&"\\".repeat(backslashes));
                out.push('"');
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// What to run to open a terminal window that runs `argv` and stays open, or None if no terminal can be found.
/// (Windows gets one string: cmd.exe wants the command line exactly as written.)
pub enum Launch {
    Raw(String),
    Argv(Vec<String>),
}

pub fn terminal_command(argv: &[String], title: &str, platform: &str) -> Option<Launch> {
    match platform {
        // `cmd /k ""C:\a b\x.exe" arg"`: cmd drops the outer pair of quotes, so a path with spaces survives
        "windows" => Some(Launch::Raw(format!("/c start \"{title}\" cmd.exe /k \"{}\"", list2cmdline(argv)))),
        "macos" => {
            let script = argv.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ").replace('\\', "\\\\").replace('"', "\\\"");
            Some(Launch::Argv(
                ["osascript", "-e", "tell application \"Terminal\" to activate", "-e", &format!("tell application \"Terminal\" to do script \"{script}\"")]
                    .map(String::from)
                    .to_vec(),
            ))
        }
        _ => LINUX_TERMINALS.iter().find(|(t, _)| which(t).is_some()).map(|(terminal, flags)| {
            let mut full = vec![terminal.to_string()];
            full.extend(flags.iter().map(|f| f.to_string()));
            full.extend(argv.iter().cloned());
            Launch::Argv(full)
        }),
    }
}

/// Run `argv` in a new terminal window that stays open (a provider's sign-in): "started", or "failed" (no terminal to open
/// it in). `GEEKMAGIC_NO_LOGIN=1` turns it off (the tests use it: no test may open a window).
pub fn launch(argv: &[String], title: &str) -> String {
    if std::env::var_os("GEEKMAGIC_NO_LOGIN").is_some() {
        return "failed".into();
    }
    let Some(command) = terminal_command(argv, title, std::env::consts::OS) else { return "failed".into() };
    let mut cmd = match command {
        #[cfg(windows)]
        Launch::Raw(line) => {
            use std::os::windows::process::CommandExt;
            let mut c = Command::new("cmd.exe");
            c.raw_arg(line);
            c
        }
        #[cfg(not(windows))]
        Launch::Raw(_) => return "failed".into(),
        Launch::Argv(full) => {
            let mut c = Command::new(&full[0]);
            c.args(&full[1..]);
            c
        }
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    match no_window(&mut cmd).spawn() {
        Ok(_) => "started".into(),
        Err(_) => "failed".into(),
    }
}

/// The TerminalLauncher port.
pub struct Terminal;

impl TerminalLauncher for Terminal {
    fn launch(&self, argv: &[String], title: &str) -> String {
        launch(argv, title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cmdline_quoting_matches_pythons() {
        assert_eq!(list2cmdline(&v(&["a", "b c", "", "d\"e"])), "a \"b c\" \"\" d\\\"e");
        assert_eq!(list2cmdline(&v(&["C:\\Program Files\\x.exe", "auth", "login"])), "\"C:\\Program Files\\x.exe\" auth login");
        assert_eq!(list2cmdline(&v(&["trailing\\", "with space\\"])), "trailing\\ \"with space\\\\\"");
    }

    #[test]
    fn windows_runs_it_in_a_cmd_that_stays_open() {
        let Some(Launch::Raw(line)) = terminal_command(&v(&["C:\\Program Files\\claude.exe", "auth", "login"]), "Claude sign-in", "windows") else {
            panic!("windows gives a raw command line")
        };
        assert_eq!(line, "/c start \"Claude sign-in\" cmd.exe /k \"\"C:\\Program Files\\claude.exe\" auth login\"");
    }

    #[test]
    fn macos_asks_terminal_to_run_it() {
        let Some(Launch::Argv(full)) = terminal_command(&v(&["/usr/local/bin/codex", "login"]), "t", "macos") else { panic!() };
        assert_eq!(full[0], "osascript");
        assert!(full[4].contains("do script \"'/usr/local/bin/codex' 'login'\""), "{}", full[4]);
    }

    #[test]
    fn the_kill_switch_never_opens_a_window() {
        std::env::set_var("GEEKMAGIC_NO_LOGIN", "1");
        assert_eq!(launch(&v(&["x"]), "t"), "failed");
    }
}
