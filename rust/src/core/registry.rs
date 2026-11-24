//! The providers in use, in the order they cycle.

use std::sync::Arc;

use super::ports::Provider;

pub struct ProviderRegistry {
    providers: Vec<Arc<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new(providers: Vec<Arc<dyn Provider>>) -> Self {
        Self { providers }
    }

    pub fn get(&self, key: &str) -> Option<&Arc<dyn Provider>> {
        self.providers.iter().find(|p| p.key() == key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Provider>> {
        self.providers.iter()
    }

    /// The keys, in order.
    pub fn keys(&self) -> Vec<String> {
        self.providers.iter().map(|p| p.key().to_string()).collect()
    }

    /// key -> display name, in order.
    pub fn titles(&self) -> Vec<(String, String)> {
        self.providers.iter().map(|p| (p.key().to_string(), p.title().to_string())).collect()
    }

    pub fn title_of(&self, key: &str) -> String {
        self.get(key).map(|p| p.title().to_string()).unwrap_or_else(|| key.to_string())
    }

    /// A reading says which provider it is by title: its key.
    pub fn key_of(&self, title: &str) -> Option<String> {
        self.providers.iter().find(|p| p.title() == title).map(|p| p.key().to_string())
    }
}
