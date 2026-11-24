//! What is on the screen: one provider's, or one of the views of two providers at once (the "mode").

use std::sync::Mutex;

use serde_json::{json, Map, Value};

use super::persistence::Part;
use crate::core::views::{is_panel_key, PANEL_KEYS};
use crate::util::LockExt;

struct Inner {
    /// A view of two providers instead of one (only the menu turns these on).
    mode: Option<String>,
    /// The last single-provider view, also what the left-click goes on from.
    provider: String,
}

pub struct ViewState {
    provider_keys: Vec<String>,
    first_run_provider: Option<String>, // --provider: only the very first run's choice
    inner: Mutex<Inner>,
}

impl ViewState {
    pub fn new(provider_keys: Vec<String>, first_run_provider: Option<String>) -> Self {
        Self { provider_keys, first_run_provider, inner: Mutex::new(Inner { mode: None, provider: "claude".into() }) }
    }

    /// What the screen should be showing: a provider, or a view of two.
    pub fn key(&self) -> String {
        let i = self.inner.lk();
        i.mode.clone().unwrap_or_else(|| i.provider.clone())
    }

    pub fn mode(&self) -> Option<String> {
        self.inner.lk().mode.clone()
    }

    pub fn provider(&self) -> String {
        self.inner.lk().provider.clone()
    }

    pub fn set_mode(&self, mode: Option<&str>) {
        self.inner.lk().mode = mode.map(String::from);
    }

    pub fn set_provider(&self, provider: &str) {
        self.inner.lk().provider = provider.to_string();
    }

    /// Show one provider (and leave any view of two).
    pub fn show_provider(&self, provider: &str) {
        let mut i = self.inner.lk();
        i.provider = provider.to_string();
        i.mode = None;
    }
}

impl Part for ViewState {
    fn name(&self) -> &'static str {
        "view"
    }

    /// The view comes back exactly as it was left; --provider is only the very first run's choice.
    fn restore(&self, saved: &Value) {
        let known = |v: Option<&Value>, extra: &[&str]| {
            v.and_then(Value::as_str).filter(|s| self.provider_keys.iter().any(|k| k == s) || extra.contains(s)).map(String::from)
        };
        let saved_view = known(saved.get("view"), &PANEL_KEYS);
        let saved_provider = known(saved.get("provider"), &[]);
        let first = self.first_run_provider.clone();
        let view = saved_view.or(first.clone()).unwrap_or_else(|| "claude".into());
        let mut i = self.inner.lk();
        if is_panel_key(&view) {
            i.provider = saved_provider.or(first).unwrap_or_else(|| "claude".into());
            i.mode = Some(view);
        } else {
            i.mode = None;
            i.provider = view;
        }
    }

    fn snapshot(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("view".into(), json!(self.key()));
        m.insert("provider".into(), json!(self.provider()));
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(first: Option<&str>) -> ViewState {
        ViewState::new(vec!["claude".into(), "codex".into()], first.map(String::from))
    }

    #[test]
    fn comes_back_to_where_it_was_left() {
        let v = view(None);
        v.restore(&json!({"view": "split", "provider": "codex"}));
        assert_eq!((v.mode().as_deref(), v.provider().as_str(), v.key().as_str()), (Some("split"), "codex", "split"));
        v.restore(&json!({"view": "codex", "provider": "claude"}));
        assert_eq!((v.mode(), v.provider()), (None, "codex".to_string()));
        assert_eq!(v.snapshot()["view"], "codex");
    }

    #[test]
    fn the_command_line_only_decides_the_first_run() {
        let v = view(Some("codex"));
        v.restore(&json!({}));
        assert_eq!(v.key(), "codex");
        v.restore(&json!({"view": "claude"}));
        assert_eq!(v.key(), "claude", "what was saved wins");
        let junk = view(None);
        junk.restore(&json!({"view": "nonsense", "provider": 4}));
        assert_eq!(junk.key(), "claude");
    }

    #[test]
    fn switching() {
        let v = view(None);
        v.set_mode(Some("stats"));
        v.show_provider("codex");
        assert_eq!((v.mode(), v.provider()), (None, "codex".to_string()));
    }
}
