//! The command line: the tray app (the default), a one-shot push of one provider to the screen, the `notify` command a hook
//! runs, and the install / uninstall of start-at-login.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use crate::adapters::device::client::{GeekMagicDevice, BRIGHTNESS_RANGE};
use crate::adapters::device::discovery;
use crate::adapters::render::animations::animation_names;
use crate::adapters::render::SoftRenderer;
use crate::adapters::system::autostart;
use crate::adapters::webhook;
use crate::application::config;
use crate::application::controller::Settings;
use crate::application::image_files;
use crate::bootstrap::{build_registry, resolve_data_dir, run_tray, TrayOptions};
use crate::core::errors::UsageError;
use crate::core::model::Data;
use crate::core::ports::{Display, Renderer};

pub const HELP: &str = "\
geekmagic-usage: Claude Code and Codex usage on a GeekMagic SmallTV, refreshed by webhooks.

USAGE
    geekmagic-usage [OPTIONS]                       the tray app (finds the screen on your network by itself)
    geekmagic-usage push [PUSH OPTIONS]             put one provider's usage on the screen, once or on a loop
    geekmagic-usage notify <provider> <event>       tell a running app something happened (what a hook runs)
    geekmagic-usage --print-hooks                   the Claude Code hooks that call it, to paste into settings.json
    geekmagic-usage --install-startup | --uninstall-startup

TRAY OPTIONS
    --ip IP                  the screen's address (omit it to find the screen on your network)
    --interval SECONDS       slow fallback refresh when no webhook arrives (default 900; 0: webhooks, clicks and
                             agent changes only)
    --provider KEY           which view to start on the very first run (claude, codex)
    --animation NAME         auto, random, or one of: {ANIMATIONS}
    --night START-END        hours of the screen's night mode, 24-hour clock, like 22-7
    --night-brightness N     backlight level during the night mode ({LOW} to {HIGH})
    --webhook-port PORT      where the webhook listens on 127.0.0.1 (default {PORT})
    --webhook-token TOKEN    require it in the X-GeekMagic-Token header (or set GEEKMAGIC_WEBHOOK_TOKEN)
    --no-webhook             do not listen at all
    --no-tray                no tray icon: run on webhooks only
    --no-discovery           never scan the network for the screen: only use --ip or the address it remembers
    --data-dir DIR           where tray.log, tray_state.json and tray.lock live (default: the project folder)

PUSH OPTIONS
    --ip IP  --provider KEY  --animation NAME  --loop SECONDS  --discover
    --out FILE.png           read the usage and save the picture instead of using a screen

WEBHOOK
    POST http://127.0.0.1:{PORT}/refresh                  read every provider again (?provider=claude for one)
    POST http://127.0.0.1:{PORT}/hook/claude/Stop         a tool says something happened
    GET  http://127.0.0.1:{PORT}/health

    --install-startup replaces the Python version's login entry (same name), --uninstall-startup removes it.
";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrayArgs {
    pub ip: Option<String>,
    pub interval: Option<f64>,
    pub provider: Option<String>,
    pub animation: Option<String>,
    pub night: Option<(i64, i64)>,
    pub night_brightness: Option<i64>,
    pub webhook_port: Option<u16>,
    pub webhook_token: Option<String>,
    pub no_webhook: bool,
    pub no_tray: bool,
    pub no_discovery: bool,
    pub data_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PushArgs {
    pub ip: Option<String>,
    pub provider: Option<String>,
    pub animation: Option<String>,
    pub loop_secs: Option<u64>,
    pub discover: bool,
    /// Save the picture here (a PNG) instead of putting it on a screen.
    pub out: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Tray(TrayArgs),
    Push(PushArgs),
    Notify { provider: String, event: String, port: u16, token: Option<String> },
    InstallStartup(TrayArgs),
    UninstallStartup,
    PrintHooks { port: u16 },
    Help,
    Version,
}

/// "22-7" -> (22, 7).
pub fn parse_night(text: &str) -> Result<(i64, i64), String> {
    let bad = || format!("expected START-END with hours 0-23, like 22-7 (got {text:?})");
    let (a, b) = text.split_once('-').ok_or_else(bad)?;
    let (start, end): (i64, i64) = (a.trim().parse().map_err(|_| bad())?, b.trim().parse().map_err(|_| bad())?);
    if !(0..=23).contains(&start) || !(0..=23).contains(&end) {
        return Err(format!("hours must be 0-23 (got {text:?})"));
    }
    Ok((start, end))
}

/// Split `--flag=value` and `--flag value` alike; the value of a flag that takes none stays unread.
struct Args {
    items: Vec<String>,
    at: usize,
}

impl Args {
    fn next_flag(&mut self) -> Option<(String, Option<String>)> {
        let item = self.items.get(self.at)?.clone();
        self.at += 1;
        match item.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => Some((flag.to_string(), Some(value.to_string()))),
            _ => Some((item, None)),
        }
    }

    fn value(&mut self, flag: &str, inline: Option<String>) -> Result<String, String> {
        if let Some(v) = inline {
            return Ok(v);
        }
        let v = self.items.get(self.at).cloned().ok_or_else(|| format!("{flag} needs a value"))?;
        self.at += 1;
        Ok(v)
    }
}

fn number<T: std::str::FromStr>(flag: &str, text: &str) -> Result<T, String> {
    text.trim().parse().map_err(|_| format!("{flag} wants a number (got {text:?})"))
}

fn check_animation(name: &str) -> Result<String, String> {
    if name == "auto" || name == "random" || animation_names().contains(&name) {
        Ok(name.to_string())
    } else {
        Err(format!("--animation: {name:?} is not auto, random or one of {}", animation_names().join(", ")))
    }
}

fn parse_tray(args: &mut Args, first: Option<(String, Option<String>)>) -> Result<(TrayArgs, Vec<String>), String> {
    let mut out = TrayArgs::default();
    let mut other = Vec::new();
    let mut pending = first;
    while let Some((flag, inline)) = pending.take().or_else(|| args.next_flag()) {
        match flag.as_str() {
            "--ip" => out.ip = Some(args.value(&flag, inline)?),
            "--interval" => {
                let v: f64 = number(&flag, &args.value(&flag, inline)?)?;
                if v < 0.0 {
                    return Err("--interval cannot be negative".into());
                }
                out.interval = Some(v);
            }
            "--provider" => {
                let v = args.value(&flag, inline)?;
                if !build_registry().contains(&v) {
                    return Err(format!("--provider: {v:?} is not one of {}", build_registry().keys().join(", ")));
                }
                out.provider = Some(v);
            }
            "--animation" => out.animation = Some(check_animation(&args.value(&flag, inline)?)?),
            "--night" => out.night = Some(parse_night(&args.value(&flag, inline)?)?),
            "--night-brightness" => {
                let v: i64 = number(&flag, &args.value(&flag, inline)?)?;
                if !(BRIGHTNESS_RANGE.0..=BRIGHTNESS_RANGE.1).contains(&v) {
                    return Err(format!("--night-brightness must be {} to {} (got {v})", BRIGHTNESS_RANGE.0, BRIGHTNESS_RANGE.1));
                }
                out.night_brightness = Some(v);
            }
            "--webhook-port" => out.webhook_port = Some(number(&flag, &args.value(&flag, inline)?)?),
            "--webhook-token" => out.webhook_token = Some(args.value(&flag, inline)?),
            "--no-webhook" => out.no_webhook = true,
            "--no-tray" => out.no_tray = true,
            "--no-discovery" => out.no_discovery = true,
            "--data-dir" => out.data_dir = Some(PathBuf::from(args.value(&flag, inline)?)),
            _ => other.push(flag),
        }
    }
    Ok((out, other))
}

fn parse_push(args: &mut Args) -> Result<PushArgs, String> {
    let mut out = PushArgs::default();
    while let Some((flag, inline)) = args.next_flag() {
        match flag.as_str() {
            "--ip" => out.ip = Some(args.value(&flag, inline)?),
            "--provider" => {
                let v = args.value(&flag, inline)?;
                if !build_registry().contains(&v) {
                    return Err(format!("--provider: {v:?} is not one of {}", build_registry().keys().join(", ")));
                }
                out.provider = Some(v);
            }
            "--animation" => out.animation = Some(check_animation(&args.value(&flag, inline)?)?),
            "--loop" => out.loop_secs = Some(number(&flag, &args.value(&flag, inline)?)?),
            "--discover" => out.discover = true,
            "--out" => out.out = Some(PathBuf::from(args.value(&flag, inline)?)),
            other => return Err(format!("unknown option {other} (see --help)")),
        }
    }
    Ok(out)
}

fn token_from_env() -> Option<String> {
    std::env::var("GEEKMAGIC_WEBHOOK_TOKEN").ok().filter(|t| !t.is_empty())
}

pub fn parse(argv: &[String]) -> Result<Command, String> {
    let mut args = Args { items: argv.to_vec(), at: 0 };
    match argv.first().map(String::as_str) {
        Some("push") => {
            args.at = 1;
            return parse_push(&mut args).map(Command::Push);
        }
        Some("notify") => {
            let mut positional = Vec::new();
            let (mut port, mut token) = (config::WEBHOOK_PORT, token_from_env());
            args.at = 1;
            while let Some((flag, inline)) = args.next_flag() {
                match flag.as_str() {
                    "--port" => port = number(&flag, &args.value(&flag, inline)?)?,
                    "--token" => token = Some(args.value(&flag, inline)?),
                    f if f.starts_with("--") => return Err(format!("unknown option {f} (see --help)")),
                    _ => positional.push(flag),
                }
            }
            return match positional.as_slice() {
                [provider, event] => Ok(Command::Notify { provider: provider.clone(), event: event.clone(), port, token }),
                _ => Err("notify wants a provider and an event: notify claude Stop".into()),
            };
        }
        _ => {}
    }
    let (mut install, mut uninstall, mut print_hooks, mut help, mut version) = (false, false, false, false, false);
    let mut tray = TrayArgs::default();
    args.at = 0;
    while let Some(flag) = args.next_flag() {
        match flag.0.as_str() {
            "--install-startup" => install = true,
            "--uninstall-startup" => uninstall = true,
            "--print-hooks" => print_hooks = true,
            "-h" | "--help" => help = true,
            "-V" | "--version" => version = true,
            _ => {
                let (parsed, other) = parse_tray(&mut args, Some(flag))?;
                if let Some(unknown) = other.first() {
                    return Err(format!("unknown option {unknown} (see --help)"));
                }
                tray = merge(tray, parsed);
            }
        }
    }
    Ok(if help {
        Command::Help
    } else if version {
        Command::Version
    } else if uninstall {
        Command::UninstallStartup
    } else if print_hooks {
        Command::PrintHooks { port: tray.webhook_port.unwrap_or(config::WEBHOOK_PORT) }
    } else if install {
        Command::InstallStartup(tray)
    } else {
        Command::Tray(tray)
    })
}

/// `later`'s choices win over `earlier`'s.
fn merge(earlier: TrayArgs, later: TrayArgs) -> TrayArgs {
    TrayArgs {
        ip: later.ip.or(earlier.ip),
        interval: later.interval.or(earlier.interval),
        provider: later.provider.or(earlier.provider),
        animation: later.animation.or(earlier.animation),
        night: later.night.or(earlier.night),
        night_brightness: later.night_brightness.or(earlier.night_brightness),
        webhook_port: later.webhook_port.or(earlier.webhook_port),
        webhook_token: later.webhook_token.or(earlier.webhook_token),
        no_webhook: later.no_webhook || earlier.no_webhook,
        no_tray: later.no_tray || earlier.no_tray,
        no_discovery: later.no_discovery || earlier.no_discovery,
        data_dir: later.data_dir.or(earlier.data_dir),
    }
}

/// The arguments the login entry should start the app with (what was asked for, so it comes back the same way).
pub fn startup_args(a: &TrayArgs) -> Vec<String> {
    let mut out = Vec::new();
    let mut add = |flag: &str, value: Option<String>| {
        if let Some(v) = value {
            out.push(flag.to_string());
            out.push(v);
        }
    };
    add("--ip", a.ip.clone());
    add("--interval", a.interval.map(|i| i.to_string()));
    add("--provider", a.provider.clone());
    add("--animation", a.animation.clone());
    add("--night", a.night.map(|(s, e)| format!("{s}-{e}")));
    add("--night-brightness", a.night_brightness.map(|n| n.to_string()));
    add("--webhook-port", a.webhook_port.map(|p| p.to_string()));
    add("--data-dir", a.data_dir.as_ref().map(|d| d.display().to_string()));
    if a.no_webhook {
        out.push("--no-webhook".into());
    }
    if a.no_tray {
        out.push("--no-tray".into());
    }
    if a.no_discovery {
        out.push("--no-discovery".into());
    }
    out
}

/// The `hooks` entry for Claude Code's settings.json that tells the app about a turn starting, waiting and ending.
pub fn hooks_json(exe: &str, port: u16) -> serde_json::Value {
    let port_arg = if port == config::WEBHOOK_PORT { String::new() } else { format!(" --port {port}") };
    let hook = |event: &str| {
        serde_json::json!([{"hooks": [{"type": "command", "command": format!("\"{exe}\" notify claude {event}{port_arg}"), "timeout": 5}]}])
    };
    serde_json::json!({"hooks": {
        "UserPromptSubmit": hook("UserPromptSubmit"),
        "Notification": hook("Notification"),
        "Stop": hook("Stop"),
        "SessionEnd": hook("SessionEnd"),
    }})
}

fn tray_options(a: TrayArgs) -> TrayOptions {
    TrayOptions {
        settings: Settings {
            ip: a.ip,
            interval: a.interval.unwrap_or(config::FALLBACK_INTERVAL),
            animation: a.animation.unwrap_or_else(|| "auto".into()),
            provider: a.provider,
            night_hours: a.night,
            night_level: a.night_brightness,
            background_stats: true,
        },
        data_dir: a.data_dir,
        webhook_port: a.webhook_port.unwrap_or(config::WEBHOOK_PORT),
        webhook: !a.no_webhook,
        webhook_token: a.webhook_token.or_else(token_from_env),
        no_tray: a.no_tray,
        discovery: !a.no_discovery,
    }
}

/// Render `usage` and put it on the screen as `filename`.
fn push_once(device: &GeekMagicDevice, renderer: &SoftRenderer, provider: &str, animation: &str, filename: &str, cleaned: &mut bool) -> Result<(), String> {
    let usage = build_registry().get(provider).ok_or("no such provider")?.fetch().map_err(|e: UsageError| e.message)?;
    let rendered = renderer.render(provider, &Data::One(usage.clone()), animation)?;
    device.upload(&rendered.gif, filename, "image/gif", None).map_err(|e| e.to_string())?;
    renderer.confirm(&rendered);
    device.show_image(filename).map_err(|e| e.to_string())?;
    if !*cleaned {
        // one-time migration cleanup, not worth an extra request every push
        for name in image_files::LEGACY_NAMES {
            let _ = device.delete_image(name);
        }
        *cleaned = true;
    }
    println!(
        "[{}] pushed [{}] ({:.1} KB) — current {}% / weekly {}%",
        usage.now.format("%H:%M:%S"),
        rendered.animations.first().map_or("", String::as_str),
        rendered.gif.len() as f64 / 1024.0,
        usage.current.pct.map_or("None".into(), |p| p.to_string()),
        usage.weekly.pct.map_or("None".into(), |p| p.to_string())
    );
    Ok(())
}

/// `push --out FILE`: read the usage and save the picture, without any screen.
fn run_preview(a: &PushArgs, out: &std::path::Path) -> i32 {
    let provider = a.provider.clone().unwrap_or_else(|| "claude".into());
    let animation = a.animation.clone().unwrap_or_else(|| "idle".into());
    let renderer = SoftRenderer::new();
    let result = build_registry()
        .get(&provider)
        .ok_or_else(|| "no such provider".to_string())
        .and_then(|p| p.fetch().map_err(|e| e.message))
        .and_then(|usage| {
            let rendered = renderer.render(&provider, &Data::One(usage.clone()), &animation)?;
            std::fs::write(out, renderer.to_png(&rendered.gif)?).map_err(|e| e.to_string())?;
            Ok((usage, rendered.gif.len()))
        });
    match result {
        Ok((usage, bytes)) => {
            println!(
                "{provider}: current {:?}% resets {:?} / weekly {:?}% resets {:?} ({:.1} KB gif) -> {}",
                usage.current.pct,
                usage.current.reset.map(|r| r.with_timezone(&chrono::Local).to_rfc3339()),
                usage.weekly.pct,
                usage.weekly.reset.map(|r| r.with_timezone(&chrono::Local).to_rfc3339()),
                bytes as f64 / 1024.0,
                out.display()
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn run_push(a: PushArgs) -> i32 {
    if let Some(out) = a.out.clone() {
        return run_preview(&a, &out);
    }
    let mut ip = a.ip.clone();
    if a.discover || ip.is_none() {
        let found = discovery::scan(None, discovery::PROBE_TIMEOUT);
        if a.discover {
            println!("{}", if found.is_empty() { "No GeekMagic device found on this network.".to_string() } else { found.join("\n") });
            return i32::from(found.is_empty());
        }
        if found.len() != 1 {
            eprintln!(
                "error: {}",
                if found.is_empty() { "no GeekMagic device found; pass --ip".to_string() } else { format!("several devices found ({}); pick one with --ip", found.join(", ")) }
            );
            return 1;
        }
        println!("found the device at {}", found[0]);
        ip = Some(found[0].clone());
    }
    let (device, renderer) = (GeekMagicDevice::new(&ip.unwrap_or_default()), SoftRenderer::new());
    let provider = a.provider.unwrap_or_else(|| "claude".into());
    let animation = a.animation.unwrap_or_else(|| "auto".into());
    let filename = image_files::image_name(&provider);
    let mut cleaned = false;
    match a.loop_secs.filter(|s| *s > 0) {
        Some(secs) => loop {
            if let Err(e) = push_once(&device, &renderer, &provider, &animation, &filename, &mut cleaned) {
                eprintln!("warning: {e}");
            }
            std::thread::sleep(Duration::from_secs(secs));
        },
        None => match push_once(&device, &renderer, &provider, &animation, &filename, &mut cleaned) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
    }
}

/// Windows: a program without a console window can still print into the terminal that started it.
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // SAFETY: a plain Win32 call; failing (no parent console) is fine.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

pub fn main() -> ExitCode {
    attach_parent_console();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse(&argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let code = match command {
        Command::Help => {
            print!(
                "{}",
                HELP.replace("{ANIMATIONS}", &animation_names().join(", "))
                    .replace("{LOW}", &BRIGHTNESS_RANGE.0.to_string())
                    .replace("{HIGH}", &BRIGHTNESS_RANGE.1.to_string())
                    .replace("{PORT}", &config::WEBHOOK_PORT.to_string())
            );
            0
        }
        Command::Version => {
            println!("geekmagic-usage {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Command::PrintHooks { port } => {
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "geekmagic-usage".into());
            println!("{}", serde_json::to_string_pretty(&hooks_json(&exe, port)).unwrap_or_default());
            0
        }
        Command::Notify { provider, event, port, token } => {
            webhook::notify(port, token.as_deref(), &provider, &event);
            0 // (a hook must never fail the tool it hooks into)
        }
        Command::InstallStartup(a) => {
            let root = resolve_data_dir(a.data_dir.clone());
            let exe = std::env::current_exe().unwrap_or_default();
            match autostart::install(&exe, &startup_args(&a), &root) {
                Ok(message) => {
                    println!("{message}");
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        Command::UninstallStartup => match autostart::uninstall() {
            Ok(message) => {
                println!("{message}");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        Command::Push(a) => run_push(a),
        Command::Tray(a) => run_tray(tray_options(a)),
    };
    ExitCode::from(code as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(items: &[&str]) -> Result<Command, String> {
        parse(&items.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn tray(items: &[&str]) -> TrayArgs {
        match p(items).unwrap() {
            Command::Tray(a) => a,
            other => panic!("not the tray: {other:?}"),
        }
    }

    #[test]
    fn the_tray_is_the_default_and_options_read_both_ways() {
        assert_eq!(tray(&[]), TrayArgs::default());
        let a = tray(&["--ip", "192.168.1.20", "--interval=0", "--provider", "codex", "--animation=dance", "--night", "22-7", "--night-brightness", "10", "--webhook-port=9000", "--no-tray"]);
        assert_eq!(a.ip.as_deref(), Some("192.168.1.20"));
        assert_eq!((a.interval, a.provider.as_deref(), a.animation.as_deref()), (Some(0.0), Some("codex"), Some("dance")));
        assert_eq!((a.night, a.night_brightness, a.webhook_port, a.no_tray), (Some((22, 7)), Some(10), Some(9000), true));
    }

    #[test]
    fn bad_options_say_what_is_wrong() {
        for (args, wants) in [
            (vec!["--night", "25-7"], "0-23"),
            (vec!["--night", "late"], "START-END"),
            (vec!["--night-brightness", "500"], "-10 to 100"),
            (vec!["--provider", "gemini"], "not one of claude, codex"),
            (vec!["--animation", "moonwalk"], "not auto, random"),
            (vec!["--interval", "-5"], "negative"),
            (vec!["--interval", "soon"], "number"),
            (vec!["--ip"], "needs a value"),
            (vec!["--bogus"], "unknown option --bogus"),
        ] {
            let e = p(&args).unwrap_err();
            assert!(e.contains(wants), "{args:?}: {e}");
        }
    }

    #[test]
    fn the_other_commands() {
        assert_eq!(p(&["--help"]).unwrap(), Command::Help);
        assert_eq!(p(&["--version"]).unwrap(), Command::Version);
        assert_eq!(p(&["--uninstall-startup"]).unwrap(), Command::UninstallStartup);
        assert_eq!(p(&["--print-hooks", "--webhook-port", "9000"]).unwrap(), Command::PrintHooks { port: 9000 });
        let Command::InstallStartup(a) = p(&["--install-startup", "--ip", "1.2.3.4", "--night", "23-6"]).unwrap() else { panic!() };
        assert_eq!(startup_args(&a), ["--ip", "1.2.3.4", "--night", "23-6"], "the login entry starts it the way it was asked for");
        assert_eq!(
            p(&["notify", "claude", "Stop", "--port", "9000"]).unwrap(),
            Command::Notify { provider: "claude".into(), event: "Stop".into(), port: 9000, token: token_from_env() }
        );
        assert!(p(&["notify", "claude"]).is_err());
        let Command::Push(push) = p(&["push", "--ip", "1.2.3.4", "--provider=codex", "--loop", "60"]).unwrap() else { panic!() };
        assert_eq!((push.ip.as_deref(), push.provider.as_deref(), push.loop_secs), (Some("1.2.3.4"), Some("codex"), Some(60)));
        assert!(p(&["push", "--bogus"]).is_err());
    }

    #[test]
    fn the_printed_hooks_call_the_notify_command_with_the_event() {
        let json = hooks_json("C:\\Tools\\geekmagic-usage.exe", config::WEBHOOK_PORT);
        let stop = &json["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(stop["type"], "command");
        assert_eq!(stop["command"], "\"C:\\Tools\\geekmagic-usage.exe\" notify claude Stop");
        assert_eq!(json["hooks"].as_object().unwrap().len(), 4);
        assert!(hooks_json("x", 9000)["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap().ends_with("notify claude Stop --port 9000"));
    }

    #[test]
    fn night_hours() {
        assert_eq!(parse_night("22-7"), Ok((22, 7)));
        assert_eq!(parse_night("0-23"), Ok((0, 23)));
        assert!(parse_night("22").is_err() && parse_night("-1-5").is_err());
    }

    #[test]
    fn the_help_names_every_option_it_parses() {
        for flag in ["--ip", "--interval", "--provider", "--animation", "--night", "--night-brightness", "--webhook-port", "--webhook-token", "--no-webhook", "--no-tray", "--no-discovery", "--data-dir", "--print-hooks", "--install-startup", "--uninstall-startup", "--loop", "--discover", "--out"] {
            assert!(HELP.contains(flag), "{flag} is not in the help");
        }
    }
}
