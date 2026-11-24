//! The screen of a provider that can't be read yet: its mascot and why.

use super::super::canvas::{text_width, Canvas};
use super::super::mascots::{draw_mascot, mascot_width, theme_for, MASCOT_TOP};
use super::super::palette::*;
use crate::core::model::ErrorScreen;

/// What a provider's screen says when it has never been read: its mascot and why (not signed in, not installed...).
/// Without it a click on a provider that can't be read would change nothing at all and look broken.
pub fn render_error_frame(screen: &ErrorScreen) -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT, BG);
    let theme = theme_for(&screen.title);
    let mascot_w = mascot_width(theme);
    draw_mascot(&mut c, 10, MASCOT_TOP, 3, theme);
    c.text(f64::from(10 + mascot_w + 10), 8.0, &screen.title, 26.0, TEXT);
    let heading = if screen.signin { "Sign-in needed" } else { "Can't read the usage" };
    c.text(10.0, 56.0, heading, 17.0, WARN);
    // the reason, word-wrapped to the screen (what a provider's own error says is meant to be read by a person)
    let message = if screen.message.trim().is_empty() { "No reading yet." } else { screen.message.as_str() };
    let (mut lines, mut line) = (Vec::<String>::new(), String::new());
    for word in message.split_whitespace() {
        let trial = format!("{line} {word}").trim().to_string();
        if text_width(&trial, 15.0) <= (WIDTH - 20) as f64 {
            line = trial;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        }
    }
    lines.push(line);
    for (i, text) in lines.iter().take(7).enumerate() {
        c.text(10.0, f64::from(86 + i as i32 * 20), text.trim(), 15.0, if i < 6 { TEXT } else { MUTED });
    }
    let footer = "Retrying every few minutes";
    c.text(((WIDTH as f64 - text_width(footer, 13.0)) / 2.0).floor(), 223.0, footer, 13.0, MUTED);
    c
}
