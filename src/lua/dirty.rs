//! Tag-based dirty tracking for efficient partial screen updates.
//!
//! Lua defines what tags exist - Rust imposes no layout assumptions.
//! This allows status at top, bottom, both sides, or any layout Lua wants.

use std::collections::HashSet;
use std::sync::RwLock;
use tokio::sync::Notify;

/// Tag-based dirty tracking with arbitrary string tags.
///
/// Rust provides this primitive; Lua composes the layout by:
/// - Defining what tags exist ("status", "chat", "input", or anything)
/// - Deciding what screen rows each tag covers
/// - Controlling when to mark tags dirty
pub struct DirtyState {
    tags: RwLock<HashSet<String>>,
    signal: Notify,
}

impl DirtyState {
    pub fn new() -> Self {
        Self {
            tags: RwLock::new(HashSet::new()),
            signal: Notify::new(),
        }
    }

    /// Mark a tag dirty and signal waiters
    pub fn mark(&self, tag: impl Into<String>) {
        let mut tags = self.tags.write().unwrap();
        tags.insert(tag.into());
        drop(tags);
        self.signal.notify_one();
    }

    /// Mark all provided tags dirty
    pub fn mark_many<I, S>(&self, new_tags: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut tags = self.tags.write().unwrap();
        for tag in new_tags {
            tags.insert(tag.into());
        }
        drop(tags);
        self.signal.notify_one();
    }

    /// Take all dirty tags, clearing the set
    pub fn take(&self) -> HashSet<String> {
        let mut tags = self.tags.write().unwrap();
        std::mem::take(&mut *tags)
    }

    /// Wait for any tag to become dirty
    pub fn notified(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.signal.notified()
    }

    /// Check if any tags are dirty (non-blocking)
    pub fn is_dirty(&self) -> bool {
        !self.tags.read().unwrap().is_empty()
    }
}

impl Default for DirtyState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_arbitrary_tags() {
        let dirty = DirtyState::new();
        dirty.mark("my-custom-region");
        dirty.mark("sidebar");

        let tags = dirty.take();
        assert!(tags.contains("my-custom-region"));
        assert!(tags.contains("sidebar"));
    }

    #[test]
    fn take_clears_tags() {
        let dirty = DirtyState::new();
        dirty.mark("foo");
        let _ = dirty.take();
        assert!(dirty.take().is_empty());
    }

    #[test]
    fn multiple_marks_same_tag_dedupes() {
        let dirty = DirtyState::new();
        dirty.mark("chat");
        dirty.mark("chat");
        dirty.mark("chat");

        let tags = dirty.take();
        assert_eq!(tags.len(), 1);
    }

    #[test]
    fn is_dirty_reflects_state() {
        let dirty = DirtyState::new();
        assert!(!dirty.is_dirty());
        dirty.mark("x");
        assert!(dirty.is_dirty());
        dirty.take();
        assert!(!dirty.is_dirty());
    }

    #[test]
    fn mark_many_works() {
        let dirty = DirtyState::new();
        dirty.mark_many(["status", "chat", "input"]);

        let tags = dirty.take();
        assert_eq!(tags.len(), 3);
        assert!(tags.contains("status"));
        assert!(tags.contains("chat"));
        assert!(tags.contains("input"));
    }
}
