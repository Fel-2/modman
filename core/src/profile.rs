//! Profiles: named, ordered, toggleable mod lists. Order = load order,
//! top (index 0) is lowest priority, bottom wins file conflicts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModEntry {
    pub slug: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub name: String,
    /// Ordered load order. Later entries override earlier ones on conflict.
    pub order: Vec<ModEntry>,
}

impl Profile {
    pub fn new(name: impl Into<String>) -> Self {
        Profile {
            name: name.into(),
            order: Vec::new(),
        }
    }

    pub fn find(&self, slug: &str) -> Option<&ModEntry> {
        self.order.iter().find(|e| e.slug == slug)
    }

    pub fn find_mut(&mut self, slug: &str) -> Option<&mut ModEntry> {
        self.order.iter_mut().find(|e| e.slug == slug)
    }

    /// Ensure a slug is present (appended, enabled) if not already tracked.
    pub fn ensure(&mut self, slug: &str) {
        if self.find(slug).is_none() {
            self.order.push(ModEntry {
                slug: slug.to_string(),
                enabled: true,
            });
        }
    }

    pub fn remove(&mut self, slug: &str) {
        self.order.retain(|e| e.slug != slug);
    }

    pub fn set_enabled(&mut self, slug: &str, enabled: bool) {
        if let Some(e) = self.find_mut(slug) {
            e.enabled = enabled;
        }
    }

    /// Move an entry from `from` index to `to` index, clamping bounds.
    pub fn move_to(&mut self, from: usize, to: usize) {
        if from >= self.order.len() {
            return;
        }
        let to = to.min(self.order.len() - 1);
        let item = self.order.remove(from);
        self.order.insert(to, item);
    }

    /// Slugs of enabled mods in load order.
    pub fn enabled_in_order(&self) -> impl Iterator<Item = &str> {
        self.order
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.slug.as_str())
    }
}
