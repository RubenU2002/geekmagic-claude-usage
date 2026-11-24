//! The AI tools the app can show. Each module implements the `Provider` port (`crate::core::ports`).

pub mod claude;
pub mod claude_logs;
pub mod codex;
pub mod codex_logs;
pub mod executables;
pub mod logs;

use std::sync::Arc;

use crate::core::ports::Provider;

/// The providers the app ships with, in the order they cycle.
pub fn default_providers() -> Vec<Arc<dyn Provider>> {
    vec![Arc::new(claude::ClaudeProvider), Arc::new(codex::CodexProvider)]
}
