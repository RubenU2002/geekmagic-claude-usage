// A tray app has no console window; the command line still prints into the terminal that started it (see cli::main).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    geekmagic_usage::cli::main()
}
