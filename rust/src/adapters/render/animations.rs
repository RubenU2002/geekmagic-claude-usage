//! The mascots' animations, and which one plays when.
//!
//! An animation is a list of frames, each `(mascot_dx, mascot_dy, extra)` where `extra` is an optional layer drawn over
//! the screen (a laptop, a mug, a bolt...). Every animation lives side by side in `ANIMATIONS`, and `animation_group`
//! says which ones each provider rotates through.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::{json, Map, Value};

use super::canvas::Canvas;
use super::mascots::MASCOT_TOP;
use super::props::{draw_bulb, draw_laptop, draw_mug, laptop_geometry, LAPTOP_BEZEL, MUG_BITMAP};
use crate::core::model::Usage;
use crate::core::themes::{hex, Rgb, CODEX_BITMAP, INK};
use crate::util::{pyround, LockExt};

pub const IDLE_BOB: [i32; 8] = [0, -1, -2, -1, 0, 1, 2, 1]; // a gentle idle bob, one loop

/// What an animation draws on top of the mascot in one frame.
#[derive(Debug, Clone)]
pub enum Layer {
    /// The laptop rising to this progress (0..1) in front of the mascot.
    Laptop(f64),
    /// A blinking cursor block sweeping across the held laptop's screen.
    Typing(i32),
    /// The mug rising beside the mascot, with steam once held (progress, steam phase).
    Mug(f64, i32),
    /// The idea spark popping in beside the mascot's head (progress, lit).
    Bulb(f64, bool),
    /// Codex's `>_` prompt on the cloud: a chevron, typed dots, a cursor block.
    Prompt { chevron: bool, dots: i32, cursor: Option<i32>, dy: i32 },
    /// Dots in the middle of a bobbing cloud: it's thinking.
    Think { dots: i32, dy: i32 },
    /// A resting `>_` with sparkles twinkling around the cloud.
    Sparkle(i32),
    /// Calm `>_`, or a lightning flash.
    Bolt(bool),
    /// `[ ... ]` brackets appearing on the cloud, with typed dots inside.
    Code { left: bool, right: bool, dots: i32 },
    /// The cloud hopping, its shadow shrinking as it rises.
    Hop { dy: i32, lift: i32 },
    Combine(Vec<Layer>),
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub dx: i32,
    pub dy: i32,
    pub extra: Option<Layer>,
}

fn f(dx: i32, dy: i32, extra: Option<Layer>) -> Frame {
    Frame { dx, dy, extra }
}
fn at(extra: Layer) -> Frame {
    f(0, 0, Some(extra))
}
fn idle_frames() -> Vec<Frame> {
    IDLE_BOB.iter().map(|dy| f(0, *dy, None)).collect()
}

const CHEVRON: [(i32, i32); 3] = [(3, 4), (4, 5), (3, 6)]; // the `>` of `>_`
const BOLT: [(i32, i32); 9] = [(8, 3), (7, 4), (8, 4), (6, 5), (7, 5), (7, 6), (8, 6), (7, 7), (6, 8)];
const BRACKET_L: [(i32, i32); 5] = [(2, 4), (3, 4), (2, 5), (2, 6), (3, 6)];
const BRACKET_R: [(i32, i32); 5] = [(11, 4), (10, 4), (11, 5), (11, 6), (10, 6)];
const SPARK_COLOR: Rgb = hex("#CFE3FF");
const SPARK_SPOTS: [(i32, i32); 5] = [(14, 3), (32, 2), (46, 5), (55, 15), (56, 27)]; // top-left px of each sparkle
const FLASH_BODY: Rgb = hex("#A9CFFF");
const BOLT_COLOR: Rgb = hex("#FFD966");
const SHADOW_COLOR: Rgb = hex("#3A352C");
const HOP_DY: [i32; 11] = [0, 0, -3, -6, -8, -8, -6, -3, 0, 2, 0];

/// One 3x3 px cell of the mascot grid; `dy` follows a bobbing/hopping cloud.
fn cell(canvas: &mut Canvas, col: i32, row: i32, color: Rgb, dy: i32) {
    let (x, y) = (10 + col * 3, MASCOT_TOP + row * 3 + dy);
    canvas.rect(x, y, x + 2, y + 2, color);
}

fn resting_prompt(canvas: &mut Canvas) {
    for (col, row) in CHEVRON.iter().chain([(7, 6), (8, 6)].iter()) {
        cell(canvas, *col, *row, INK, 0);
    }
}

/// Draw one layer. `mascot_w` is the mascot's width in pixels.
pub fn draw_layer(canvas: &mut Canvas, layer: &Layer, mascot_w: i32) {
    match layer {
        Layer::Laptop(progress) => {
            if *progress <= 0.0 {
                return;
            }
            let (x, hidden_y, held_y) = laptop_geometry(mascot_w);
            let y = pyround(f64::from(hidden_y) - f64::from(hidden_y - held_y) * progress) as i32;
            draw_laptop(canvas, x, y, 3);
        }
        Layer::Typing(cursor_col) => {
            let (x, _, held_y) = laptop_geometry(mascot_w);
            let col = 1 + (cursor_col % 6);
            let (cx, cy) = (x + col * 3, held_y + 3);
            canvas.rect(cx, cy, cx + 2, cy + 5, LAPTOP_BEZEL);
        }
        Layer::Mug(progress, steam_phase) => {
            if *progress <= 0.0 {
                return;
            }
            let mug_cell = 2;
            let mug_w = mug_cell * MUG_BITMAP[0].len() as i32;
            let x = 10 + mascot_w - mug_w + 2;
            let (hidden_y, held_y) = (34, 15);
            let y = pyround(f64::from(hidden_y) - f64::from(hidden_y - held_y) * progress) as i32;
            draw_mug(canvas, x, y, mug_cell, if *progress >= 1.0 { *steam_phase } else { 6 });
        }
        Layer::Bulb(progress, lit) => {
            if *progress <= 0.0 {
                return;
            }
            let x = 10 + mascot_w - 8;
            let (held_y, hidden_y) = (2, 14);
            let y = pyround(f64::from(hidden_y) - f64::from(hidden_y - held_y) * progress) as i32;
            draw_bulb(canvas, x, y, 2, *lit);
        }
        Layer::Prompt { chevron, dots, cursor, dy } => {
            if *chevron {
                for (col, row) in CHEVRON {
                    cell(canvas, col, row, INK, *dy);
                }
            }
            for i in 0..*dots {
                cell(canvas, 7 + i * 2, 6, INK, *dy); // dots sit on the baseline, like periods
            }
            if let Some(c) = cursor {
                for col in [*c, *c + 1] {
                    cell(canvas, col, 6, INK, *dy);
                }
            }
        }
        Layer::Think { dots, dy } => {
            for i in 0..*dots {
                cell(canvas, 5 + 2 * i, 5, INK, *dy);
            }
        }
        Layer::Sparkle(phase) => {
            resting_prompt(canvas);
            for (i, (x, y)) in SPARK_SPOTS.iter().enumerate() {
                let stage = (phase + i as i32) % 4;
                if stage == 0 {
                    continue;
                }
                canvas.rect(x + 2, y + 2, x + 3, y + 3, SPARK_COLOR); // centre dot
                if stage == 2 {
                    for (ax, ay) in [(x + 2, *y), (x + 2, y + 4), (*x, y + 2), (x + 4, y + 2)] {
                        canvas.rect(ax, ay, ax + 1, ay + 1, SPARK_COLOR); // plus-shaped arms
                    }
                }
            }
        }
        Layer::Bolt(flash) => {
            if !flash {
                resting_prompt(canvas);
                return;
            }
            for (row, line) in CODEX_BITMAP.iter().enumerate() {
                for (col, ch) in line.chars().enumerate() {
                    if ch == '1' {
                        cell(canvas, col as i32, row as i32, FLASH_BODY, 0);
                    }
                }
            }
            for (col, row) in BOLT {
                cell(canvas, col, row, BOLT_COLOR, 0);
            }
        }
        Layer::Code { left, right, dots } => {
            if *left {
                for (col, row) in BRACKET_L {
                    cell(canvas, col, row, INK, 0);
                }
            }
            if *right {
                for (col, row) in BRACKET_R {
                    cell(canvas, col, row, INK, 0);
                }
            }
            for i in 0..*dots {
                cell(canvas, 4 + 2 * i, 5, INK, 0);
            }
        }
        Layer::Hop { dy, lift } => {
            let half = 8.max(17 - lift);
            canvas.rect(31 - half, 36, 31 + half, 37, SHADOW_COLOR);
            for (col, row) in CHEVRON.iter().chain([(7, 6), (8, 6)].iter()) {
                cell(canvas, *col, *row, INK, *dy);
            }
        }
        Layer::Combine(layers) => {
            for layer in layers {
                draw_layer(canvas, layer, mascot_w);
            }
        }
    }
}

fn typing_frame(col: i32) -> Frame {
    at(Layer::Combine(vec![Layer::Laptop(1.0), Layer::Typing(col)]))
}

fn build() -> Vec<(&'static str, Vec<Frame>)> {
    let mut all: Vec<(&'static str, Vec<Frame>)> = Vec::new();
    all.push(("idle", idle_frames()));

    let mut laptop = idle_frames();
    laptop.extend([at(Layer::Laptop(0.35)), at(Layer::Laptop(0.7)), at(Layer::Laptop(1.0))]);
    laptop.extend([1, 2, 1, 0].map(|dy| f(0, dy, Some(Layer::Laptop(1.0)))));
    laptop.push(at(Layer::Laptop(0.5)));
    all.push(("laptop", laptop));

    let mut coffee = idle_frames();
    coffee.extend([at(Layer::Mug(0.4, 0)), at(Layer::Mug(0.7, 0))]);
    coffee.extend([0, 1, 2, 3, 4, 5, 0].map(|steam| at(Layer::Mug(1.0, steam))));
    coffee.push(at(Layer::Mug(0.5, 0)));
    all.push(("coffee", coffee));

    let mut eureka = idle_frames();
    eureka.extend([
        at(Layer::Bulb(0.5, true)),
        at(Layer::Bulb(1.0, true)),
        at(Layer::Bulb(1.0, false)),
        at(Layer::Bulb(1.0, true)),
        at(Layer::Bulb(1.0, false)),
        at(Layer::Bulb(1.0, true)),
        at(Layer::Bulb(0.5, true)),
    ]);
    all.push(("eureka", eureka));

    all.push((
        "dance",
        [(0, 0), (2, -2), (3, 0), (2, 2), (0, 0), (-2, -2), (-3, 0), (-2, 2)].map(|(dx, dy)| f(dx, dy, None)).to_vec(),
    ));

    let mut typing = idle_frames();
    typing.extend([at(Layer::Laptop(0.35)), at(Layer::Laptop(0.7))]);
    typing.extend((0..6).map(typing_frame));
    typing.extend([at(Layer::Laptop(1.0)), at(Layer::Laptop(0.5))]);
    all.push(("typing", typing));

    // Codex: the prompt appears, the cursor blinks, then it types three dots
    let prompt = |chevron: bool, dots: i32, cursor: Option<i32>| at(Layer::Prompt { chevron, dots, cursor, dy: 0 });
    let mut p = vec![f(0, 0, None), f(0, 0, None), prompt(true, 0, None)];
    p.extend([prompt(true, 0, Some(7)), prompt(true, 0, Some(7)), prompt(true, 0, None), prompt(true, 0, None)]);
    p.extend([prompt(true, 0, Some(7)), prompt(true, 0, Some(7))]);
    p.extend([prompt(true, 1, Some(9)), prompt(true, 1, Some(9)), prompt(true, 2, Some(11)), prompt(true, 2, Some(11))]);
    p.extend([prompt(true, 3, None), prompt(true, 3, None), prompt(true, 3, None), prompt(true, 0, None), prompt(true, 0, None)]);
    all.push(("prompt", p));

    // bobbing cloud (one bob cycle = smaller GIF), dots 0..3 cycling
    all.push((
        "think",
        (0..8).map(|i| f(0, IDLE_BOB[i], Some(Layer::Think { dots: ((i / 2) % 4) as i32, dy: IDLE_BOB[i] }))).collect(),
    ));
    all.push(("sparkle", (0..8).map(|i| at(Layer::Sparkle(i))).collect()));

    let mut bolt: Vec<Frame> = vec![at(Layer::Bolt(false)); 5];
    bolt.extend([at(Layer::Bolt(true)), at(Layer::Bolt(true)), at(Layer::Bolt(false)), at(Layer::Bolt(true))]);
    bolt.extend(vec![at(Layer::Bolt(false)); 4]);
    all.push(("bolt", bolt));

    // [ ] brackets appear, three dots get typed inside
    let code = |left: bool, right: bool, dots: i32| at(Layer::Code { left, right, dots });
    all.push((
        "code",
        vec![
            f(0, 0, None), f(0, 0, None),
            code(true, false, 0), code(true, false, 0),
            code(true, true, 0), code(true, true, 0),
            code(true, true, 1), code(true, true, 1),
            code(true, true, 2), code(true, true, 2),
            code(true, true, 3), code(true, true, 3), code(true, true, 3),
            code(true, true, 0), code(true, true, 0),
        ],
    ));

    all.push(("hop", HOP_DY.iter().map(|dy| f(0, *dy, Some(Layer::Hop { dy: *dy, lift: -dy }))).collect()));
    all
}

static ANIMATION_TABLE: LazyLock<Vec<(&'static str, Vec<Frame>)>> = LazyLock::new(build);

/// The frames of a named animation.
pub fn animation(name: &str) -> Option<&'static [Frame]> {
    ANIMATION_TABLE.iter().find(|(n, _)| *n == name).map(|(_, frames)| frames.as_slice())
}

/// Every animation name, in the order they were defined.
pub fn animation_names() -> Vec<&'static str> {
    ANIMATION_TABLE.iter().map(|(n, _)| *n).collect()
}

pub const DEFAULT_ANIMATION: &str = "laptop";

/// Each provider rotates through its own set; "auto"/"random" (the default) pick one, never the same twice in a row.
pub fn animation_group(title: &str) -> &'static [&'static str] {
    match title {
        "Codex" => &["prompt", "think", "sparkle", "bolt", "code", "hop"],
        _ => &["idle", "laptop", "coffee", "eureka", "dance", "typing"],
    }
}

pub const RECENT_ANIMATIONS: usize = 3; // an animation can't come back until this many different ones have played

fn working_animation(title: &str) -> &'static str {
    if title == "Codex" { "code" } else if title == "Claude" { "typing" } else { "idle" }
}
fn waiting_animation(title: &str) -> &'static str {
    if title == "Codex" { "sparkle" } else if title == "Claude" { "eureka" } else { "idle" }
}

/// Which animations each provider played lately, so a pick never repeats one that is still fresh in memory. The history
/// only grows once an upload succeeds (`record`), so a cancelled or failed upload can't make the next pick repeat
/// something that is actually on screen.
pub struct Rotation {
    recent: Mutex<HashMap<String, Vec<String>>>,
    rng: Mutex<u64>,
}

impl Default for Rotation {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64 | 1);
        Self { recent: Mutex::new(HashMap::new()), rng: Mutex::new(seed) }
    }
}

impl Rotation {
    fn next_random(&self) -> u64 {
        let mut s = self.rng.lk();
        *s ^= *s << 13; // xorshift64
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    /// A random animation from the provider's own set, none of the last RECENT_ANIMATIONS shown.
    pub fn pick(&self, title: &str) -> &'static str {
        let group = animation_group(title);
        let keep_out = RECENT_ANIMATIONS.min(group.len() - 1);
        let recent: Vec<String> = {
            let all = self.recent.lk();
            let history = all.get(title).cloned().unwrap_or_default();
            history[history.len().saturating_sub(keep_out)..].to_vec()
        };
        let candidates: Vec<&'static str> = group.iter().copied().filter(|a| !recent.iter().any(|r| r == a)).collect();
        candidates[(self.next_random() % candidates.len() as u64) as usize]
    }

    pub fn record(&self, title: &str, name: &str) {
        let mut all = self.recent.lk();
        let history = all.entry(title.to_string()).or_default();
        history.push(name.to_string());
        let extra = history.len().saturating_sub(RECENT_ANIMATIONS);
        history.drain(..extra);
    }

    pub fn restore(&self, saved: &Value) {
        let Some(animations) = saved.get("animations").and_then(Value::as_object) else { return };
        let mut all = self.recent.lk();
        for (title, names) in animations {
            let names: Vec<&str> = match names {
                Value::String(one) => vec![one.as_str()], // older files stored just the last one
                Value::Array(list) => list.iter().filter_map(Value::as_str).collect(),
                _ => continue,
            };
            let known: Vec<String> = names.into_iter().filter(|n| animation(n).is_some()).map(String::from).collect();
            let skip = known.len().saturating_sub(RECENT_ANIMATIONS);
            all.insert(title.clone(), known[skip..].to_vec());
        }
    }

    pub fn snapshot(&self) -> Map<String, Value> {
        let all = self.recent.lk();
        let animations: Map<String, Value> = all.iter().map(|(t, names)| (t.clone(), json!(names))).collect();
        let mut out = Map::new();
        out.insert("animations".into(), Value::Object(animations));
        out
    }
}

/// What the mascot does while its agent is busy (None when it isn't).
pub fn busy_animation(usage: &Usage) -> Option<&'static str> {
    if usage.waiting {
        Some(waiting_animation(&usage.title))
    } else if usage.working {
        Some(working_animation(&usage.title))
    } else {
        None
    }
}

/// The animation a provider's screen should play: the one asked for if it's one of that provider's, else (for "auto" /
/// "random") what its busy agent calls for, else a random one from its own set.
pub fn animation_for(usage: &Usage, requested: &str, rotation: &Rotation) -> String {
    let group = animation_group(&usage.title);
    if let Some(busy) = busy_animation(usage) {
        if requested == "auto" || requested == "random" {
            return busy.to_string();
        }
    }
    if group.contains(&requested) {
        requested.to_string()
    } else {
        rotation.pick(&usage.title).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Window;

    fn usage(title: &str, working: bool, waiting: bool) -> Usage {
        let mut u = Usage::new(title, Window::default(), Window::default(), chrono::Local::now());
        u.working = working;
        u.waiting = waiting;
        u
    }

    #[test]
    fn every_animation_exists_with_its_frame_count() {
        assert_eq!(
            animation_names(),
            ["idle", "laptop", "coffee", "eureka", "dance", "typing", "prompt", "think", "sparkle", "bolt", "code", "hop"]
        );
        let counts: Vec<usize> = animation_names().iter().map(|n| animation(n).unwrap().len()).collect();
        assert_eq!(counts, [8, 16, 18, 15, 8, 18, 18, 8, 8, 13, 15, 11]);
        for group in [animation_group("Claude"), animation_group("Codex")] {
            assert!(group.iter().all(|n| animation(n).is_some()));
        }
    }

    #[test]
    fn a_pick_never_repeats_what_is_fresh() {
        let r = Rotation::default();
        for _ in 0..50 {
            let a = r.pick("Claude");
            assert!(animation_group("Claude").contains(&a));
            r.record("Claude", a);
            let recent = r.recent.lk()["Claude"].clone();
            assert!(recent.len() <= RECENT_ANIMATIONS);
        }
        for _ in 0..4 {
            let before = r.recent.lk().get("Codex").cloned().unwrap_or_default();
            let a = r.pick("Codex");
            assert!(!before.iter().any(|b| b == a), "{a} was played in {before:?}");
            r.record("Codex", a);
        }
    }

    #[test]
    fn busy_agents_choose_the_animation() {
        let r = Rotation::default();
        assert_eq!(animation_for(&usage("Claude", true, false), "auto", &r), "typing");
        assert_eq!(animation_for(&usage("Claude", true, true), "auto", &r), "eureka");
        assert_eq!(animation_for(&usage("Codex", true, false), "random", &r), "code");
        assert_eq!(animation_for(&usage("Codex", true, true), "auto", &r), "sparkle");
        assert_eq!(animation_for(&usage("Claude", false, false), "dance", &r), "dance");
        assert!(animation_group("Codex").contains(&animation_for(&usage("Codex", false, false), "dance", &r).as_str()));
    }

    #[test]
    fn rotation_is_remembered_across_restarts_and_unknown_names_dropped() {
        let r = Rotation::default();
        r.restore(&json!({"animations": {"Claude": ["laptop", "gone", "dance"], "Codex": "bolt"}}));
        let snap = r.snapshot();
        assert_eq!(snap["animations"]["Claude"], json!(["laptop", "dance"]));
        assert_eq!(snap["animations"]["Codex"], json!(["bolt"]));
    }
}
