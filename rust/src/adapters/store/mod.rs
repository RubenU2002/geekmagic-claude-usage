//! What the app remembers between runs, in one JSON file.
//!
//! Each part of the app that has something to remember implements `restore(saved)` and `snapshot()`; this only reads and
//! writes the file, so a damaged one costs nothing but the memory. The file has the same keys the Python version writes,
//! so either app can pick up where the other left off.

use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::core::ports::StateStore;

pub struct JsonStateStore {
    path: PathBuf,
}

impl JsonStateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl StateStore for JsonStateStore {
    fn load(&self) -> Map<String, Value> {
        match std::fs::read_to_string(&self.path).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()) {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        }
    }

    fn save(&self, state: &Map<String, Value>) {
        // written beside the file and moved over it, so a crash mid-write can't leave half a file behind
        let temp = self.path.with_extension("json.tmp");
        let written = serde_json::to_vec(state).ok().is_some_and(|bytes| std::fs::write(&temp, bytes).is_ok());
        if !written || std::fs::rename(&temp, &self.path).is_err() {
            log::warn!("could not save {}", self.path.display());
            let _ = std::fs::remove_file(&temp);
        }
    }

    fn path(&self) -> String {
        self.path.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gm-store-{name}-{}.json", std::process::id()))
    }

    #[test]
    fn saves_and_loads() {
        let p = path("roundtrip");
        let store = JsonStateStore::new(&p);
        assert!(store.load().is_empty(), "nothing there yet");
        let state = json!({"view": "claude", "history": {"claude/current": [[1.0, 2.0, 3.0]]}});
        store.save(state.as_object().unwrap());
        assert_eq!(Value::Object(store.load()), state);
        store.save(json!({"view": "codex"}).as_object().unwrap());
        assert_eq!(store.load()["view"], "codex", "overwritten");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_damaged_file_is_just_empty() {
        let p = path("damaged");
        std::fs::write(&p, "{ not json").unwrap();
        assert!(JsonStateStore::new(&p).load().is_empty());
        std::fs::write(&p, "[1, 2]").unwrap();
        assert!(JsonStateStore::new(&p).load().is_empty(), "not an object");
        let _ = std::fs::remove_file(&p);
    }
}
