//! In-memory ledger for conversation entries
//!
//! Raw data storage with no formatting. Each entry captures who said what and when.

use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Unique identifier for a ledger entry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntryId(pub u64);

/// Who produced this entry
#[derive(Debug, Clone)]
pub enum EntrySource {
    /// User input/chat
    User(String),
    /// Model response
    Model { name: String, is_streaming: bool },
    /// System message (join, leave, errors)
    System,
    /// Command output
    Command { command: String },
}

/// Status indicator types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusKind {
    /// Invisible placeholder - will be replaced with content
    Pending,
    Thinking,
    /// Running a tool, with optional tool name
    RunningTool(Option<String>),
    Connecting,
    Complete,
}

/// Presence action types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceAction {
    Join,
    Leave,
}

/// The type of content in the entry
#[derive(Debug, Clone)]
pub enum EntryContent {
    /// Plain chat message
    Chat(String),
    /// Multi-line command output
    CommandOutput(String),
    /// Status indicator (thinking, running tool, etc.)
    Status(StatusKind),
    /// Room header/banner
    RoomHeader {
        name: String,
        description: Option<String>,
    },
    /// Welcome banner
    Welcome { username: String },
    /// History separator
    HistorySeparator { label: String },
    /// Error message
    Error(String),
    /// Join/Leave notification
    Presence {
        user: String,
        action: PresenceAction,
    },
    /// Compaction marker - summarizes older history
    /// Entries before a Compaction can be dropped from context (they're summarized)
    Compaction(String),
}

/// A single entry in the display ledger
#[derive(Debug, Clone)]
pub struct LedgerEntry {
    pub id: EntryId,
    pub timestamp: DateTime<Utc>,
    pub source: EntrySource,
    pub content: EntryContent,
    /// If true, this entry can be updated (streaming, status)
    pub mutable: bool,
    /// If true, this entry is displayed but excluded from context/history
    /// Used for debug output like /wrap, system info, etc.
    pub ephemeral: bool,
    /// True if this entry should be collapsed with adjacent blanks
    pub collapsible: bool,
}

/// The in-memory ledger of conversation entries
pub struct Ledger {
    entries: Vec<LedgerEntry>,
    next_id: u64,
    capacity: usize,
}

impl Ledger {
    /// Create a new ledger with the given capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity.min(1024)),
            next_id: 0,
            capacity,
        }
    }

    /// Add a new entry, returns its ID
    pub fn push(&mut self, source: EntrySource, content: EntryContent) -> EntryId {
        let id = EntryId(self.next_id);
        self.next_id += 1;

        self.entries.push(LedgerEntry {
            id,
            timestamp: Utc::now(),
            source,
            content,
            mutable: false,
            ephemeral: false,
            collapsible: true,
        });

        // Trim to capacity (ring buffer behavior)
        if self.entries.len() > self.capacity {
            self.entries.remove(0);
        }

        id
    }

    /// Add a mutable entry (for streaming/status placeholders)
    pub fn push_mutable(&mut self, source: EntrySource, content: EntryContent) -> EntryId {
        let id = EntryId(self.next_id);
        self.next_id += 1;

        self.entries.push(LedgerEntry {
            id,
            timestamp: Utc::now(),
            source,
            content,
            mutable: true,
            ephemeral: false,
            collapsible: false, // Placeholders shouldn't be collapsed
        });

        if self.entries.len() > self.capacity {
            self.entries.remove(0);
        }

        id
    }

    /// Add an ephemeral entry (displayed but excluded from context/history)
    /// Used for debug output like /wrap, system info, etc.
    pub fn push_ephemeral(&mut self, source: EntrySource, content: EntryContent) -> EntryId {
        let id = EntryId(self.next_id);
        self.next_id += 1;

        self.entries.push(LedgerEntry {
            id,
            timestamp: Utc::now(),
            source,
            content,
            mutable: false,
            ephemeral: true,
            collapsible: true,
        });

        if self.entries.len() > self.capacity {
            self.entries.remove(0);
        }

        id
    }

    /// Toggle ephemeral status of an entry by ID
    /// Returns true if found and toggled
    pub fn toggle_ephemeral(&mut self, id: EntryId) -> Option<bool> {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.ephemeral = !entry.ephemeral;
            Some(entry.ephemeral)
        } else {
            None
        }
    }

    /// Set ephemeral status of an entry by ID
    pub fn set_ephemeral(&mut self, id: EntryId, ephemeral: bool) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.ephemeral = ephemeral;
            true
        } else {
            false
        }
    }

    /// Update a mutable entry's content
    pub fn update(&mut self, id: EntryId, content: EntryContent) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.mutable) {
            entry.content = content;
            true
        } else {
            false
        }
    }

    /// Finalize a mutable entry (no more updates)
    pub fn finalize(&mut self, id: EntryId) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.mutable = false;
        }
    }

    /// Append text to a mutable Chat entry (for streaming)
    ///
    /// If the entry is Status, converts it to Chat first.
    /// Returns false if entry not found or not mutable.
    pub fn append(&mut self, id: EntryId, text: &str) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.mutable) {
            match &mut entry.content {
                EntryContent::Chat(existing) => {
                    existing.push_str(text);
                    true
                }
                EntryContent::Status(_) => {
                    // Convert status to chat with the new text
                    entry.content = EntryContent::Chat(text.to_string());
                    true
                }
                _ => false,
            }
        } else {
            false
        }
    }

    /// Set status on a mutable entry (for tool calls during streaming)
    pub fn set_status(&mut self, id: EntryId, status: StatusKind) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.mutable) {
            entry.content = EntryContent::Status(status);
            true
        } else {
            false
        }
    }

    /// Get entries since a given ID (exclusive)
    pub fn since(&self, after: Option<EntryId>) -> &[LedgerEntry] {
        match after {
            None => &self.entries,
            Some(id) => {
                let idx = self.entries.iter().position(|e| e.id == id);
                match idx {
                    Some(i) if i + 1 < self.entries.len() => &self.entries[i + 1..],
                    _ => &[],
                }
            }
        }
    }

    /// Get the last entry ID
    pub fn last_id(&self) -> Option<EntryId> {
        self.entries.last().map(|e| e.id)
    }

    /// Get all entries
    pub fn all(&self) -> &[LedgerEntry] {
        &self.entries
    }

    /// Get a specific entry by ID
    pub fn get(&self, id: EntryId) -> Option<&LedgerEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Check if an entry is mutable
    pub fn is_mutable(&self, id: EntryId) -> bool {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.mutable)
            .unwrap_or(false)
    }

    /// Get count of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if ledger is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Get the N most recent entries
    pub fn recent(&self, count: usize) -> &[LedgerEntry] {
        let start = self.entries.len().saturating_sub(count);
        &self.entries[start..]
    }

    /// Get non-ephemeral entries for context building
    /// Skips ephemeral entries and returns only those suitable for model context
    pub fn context_entries(&self) -> impl Iterator<Item = &LedgerEntry> {
        self.entries.iter().filter(|e| !e.ephemeral)
    }

    /// Insert a compaction marker that summarizes older entries
    /// Returns the ID of the compaction entry
    pub fn compact(&mut self, summary: String) -> EntryId {
        self.push(EntrySource::System, EntryContent::Compaction(summary))
    }
}

/// Tracks pending placeholders and their line positions
#[derive(Debug, Default)]
pub struct PlaceholderTracker {
    /// Map from entry ID to line offset
    pending: HashMap<EntryId, usize>,
    /// Lines output since last placeholder
    lines_since_last: usize,
}

impl PlaceholderTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new placeholder at the current position
    pub fn register(&mut self, id: EntryId) {
        self.pending.insert(id, 0);
    }

    /// Increment line count (call when outputting lines)
    pub fn add_lines(&mut self, count: usize) {
        self.lines_since_last += count;
        // Update all pending placeholders
        for offset in self.pending.values_mut() {
            *offset += count;
        }
    }

    /// Get the line offset for a placeholder (how many lines to go back)
    pub fn get_offset(&self, id: EntryId) -> Option<usize> {
        self.pending.get(&id).copied()
    }

    /// Remove a placeholder (after it's been resolved)
    pub fn remove(&mut self, id: EntryId) -> Option<usize> {
        self.pending.remove(&id)
    }

    /// Check if there are pending placeholders
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ledger_push() {
        let mut ledger = Ledger::new(100);

        let id1 = ledger.push(EntrySource::User("alice".into()), EntryContent::Chat("hello".into()));
        let id2 = ledger.push(
            EntrySource::Model {
                name: "qwen".into(),
                is_streaming: false,
            },
            EntryContent::Chat("hi there".into()),
        );

        assert_eq!(ledger.len(), 2);
        assert_eq!(id1, EntryId(0));
        assert_eq!(id2, EntryId(1));
    }

    #[test]
    fn test_ledger_mutable() {
        let mut ledger = Ledger::new(100);

        let id = ledger.push_mutable(
            EntrySource::Model {
                name: "qwen".into(),
                is_streaming: false,
            },
            EntryContent::Status(StatusKind::Thinking),
        );

        assert!(ledger.is_mutable(id));

        // Update the content
        assert!(ledger.update(id, EntryContent::Chat("response".into())));

        // Finalize
        ledger.finalize(id);
        assert!(!ledger.is_mutable(id));

        // Can't update after finalize
        assert!(!ledger.update(id, EntryContent::Chat("new".into())));
    }

    #[test]
    fn test_ledger_since() {
        let mut ledger = Ledger::new(100);

        let id1 = ledger.push(EntrySource::System, EntryContent::Chat("first".into()));
        let _id2 = ledger.push(EntrySource::System, EntryContent::Chat("second".into()));
        let _id3 = ledger.push(EntrySource::System, EntryContent::Chat("third".into()));

        let since = ledger.since(Some(id1));
        assert_eq!(since.len(), 2);
    }

    #[test]
    fn test_ledger_capacity() {
        let mut ledger = Ledger::new(3);

        ledger.push(EntrySource::System, EntryContent::Chat("1".into()));
        ledger.push(EntrySource::System, EntryContent::Chat("2".into()));
        ledger.push(EntrySource::System, EntryContent::Chat("3".into()));
        ledger.push(EntrySource::System, EntryContent::Chat("4".into()));

        assert_eq!(ledger.len(), 3);
        // First entry should be gone
        if let EntryContent::Chat(s) = &ledger.all()[0].content {
            assert_eq!(s, "2");
        } else {
            panic!("Expected Chat");
        }
    }

    #[test]
    fn test_placeholder_tracker() {
        let mut tracker = PlaceholderTracker::new();

        tracker.register(EntryId(1));
        assert!(tracker.has_pending());

        tracker.add_lines(3);
        assert_eq!(tracker.get_offset(EntryId(1)), Some(3));

        tracker.register(EntryId(2));
        tracker.add_lines(2);

        assert_eq!(tracker.get_offset(EntryId(1)), Some(5));
        assert_eq!(tracker.get_offset(EntryId(2)), Some(2));

        tracker.remove(EntryId(1));
        assert_eq!(tracker.get_offset(EntryId(1)), None);
    }
}
