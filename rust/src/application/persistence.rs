//! Saving and restoring the parts of the app that have something to remember, as one file.

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};

use crate::core::ports::{Renderer, StateStore};
use crate::util::LockExt;

/// Something with a memory: it gets its piece of the file back, and says what to write.
pub trait Part: Send + Sync {
    fn name(&self) -> &'static str;
    fn restore(&self, saved: &Value);
    fn snapshot(&self) -> Map<String, Value>;
}

pub struct Persistence {
    store: Arc<dyn StateStore>,
    parts: Mutex<Vec<Arc<dyn Part>>>,
    writing: Mutex<()>,
}

impl Persistence {
    pub fn new(store: Arc<dyn StateStore>) -> Self {
        Self { store, parts: Mutex::new(Vec::new()), writing: Mutex::new(()) }
    }

    pub fn register(&self, part: Arc<dyn Part>) {
        self.parts.lk().push(part);
    }

    /// Hand what was saved to every part. A part that can't make sense of its piece starts afresh; the others aren't
    /// affected (each reads defensively, so one bad value costs only itself).
    pub fn load(&self) {
        let saved = Value::Object(self.store.load());
        let parts = self.parts.lk().clone();
        for part in parts {
            part.restore(&saved);
        }
    }

    /// Write every part's memory. Never call it while holding a part's own lock: the parts are asked for their snapshot.
    pub fn save(&self) {
        let parts = self.parts.lk().clone();
        let _one_writer = self.writing.lk();
        let mut state = Map::new();
        for part in &parts {
            state.extend(part.snapshot());
        }
        self.store.save(&state);
    }
}

/// The renderer remembers which animations played lately.
pub struct RendererPart(pub Arc<dyn Renderer>);

impl Part for RendererPart {
    fn name(&self) -> &'static str {
        "renderer"
    }
    fn restore(&self, saved: &Value) {
        self.0.restore(saved);
    }
    fn snapshot(&self) -> Map<String, Value> {
        self.0.snapshot()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    /// A store that keeps its state in memory.
    #[derive(Default)]
    pub struct MemoryStore {
        pub state: Mutex<Map<String, Value>>,
        pub saves: Mutex<usize>,
    }

    impl StateStore for MemoryStore {
        fn load(&self) -> Map<String, Value> {
            self.state.lk().clone()
        }
        fn save(&self, state: &Map<String, Value>) {
            *self.state.lk() = state.clone();
            *self.saves.lk() += 1;
        }
        fn path(&self) -> String {
            "memory".into()
        }
    }

    struct Counter {
        value: Mutex<i64>,
        key: &'static str,
    }

    impl Part for Counter {
        fn name(&self) -> &'static str {
            "counter"
        }
        fn restore(&self, saved: &Value) {
            *self.value.lk() = saved.get(self.key).and_then(Value::as_i64).unwrap_or(0);
        }
        fn snapshot(&self) -> Map<String, Value> {
            let mut m = Map::new();
            m.insert(self.key.into(), json!(*self.value.lk()));
            m
        }
    }

    #[test]
    fn parts_get_their_piece_back_and_a_bad_piece_costs_only_itself() {
        let store = Arc::new(MemoryStore::default());
        store.state.lk().extend(json!({"a": 5, "b": "not a number"}).as_object().unwrap().clone());
        let p = Persistence::new(store.clone());
        let (a, b) = (Arc::new(Counter { value: Mutex::new(0), key: "a" }), Arc::new(Counter { value: Mutex::new(0), key: "b" }));
        p.register(a.clone());
        p.register(b.clone());
        p.load();
        assert_eq!((*a.value.lk(), *b.value.lk()), (5, 0));
        *a.value.lk() = 9;
        p.save();
        assert_eq!(Value::Object(store.state.lk().clone()), json!({"a": 9, "b": 0}));
    }
}
