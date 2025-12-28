//! Display layer for terminal output
//!
//! Separates raw conversation data from formatting. The Ledger stores entries,
//! the DisplayBuffer tracks render state, and the renderer formats output.
//! The HUD module provides the composable heads-up display at the bottom.

pub mod hud;
mod ledger;
mod renderer;
pub mod styles;

pub use ledger::{
    EntryContent, EntryId, EntrySource, Ledger, LedgerEntry, PlaceholderTracker, PresenceAction,
    StatusKind,
};
pub use renderer::{count_lines, render_entries, render_entry, RenderConfig};

/// Tracks what has been rendered and manages placeholder updates
pub struct DisplayBuffer {
    /// Last rendered entry ID (for incremental rendering)
    last_rendered: Option<EntryId>,
    /// Rendering configuration
    config: RenderConfig,
    /// Track pending placeholders and their line offsets
    placeholders: PlaceholderTracker,
}

impl DisplayBuffer {
    /// Create a new display buffer with the given terminal width
    pub fn new(width: u16) -> Self {
        Self {
            last_rendered: None,
            config: RenderConfig {
                width,
                ..Default::default()
            },
            placeholders: PlaceholderTracker::new(),
        }
    }

    /// Update terminal width
    pub fn set_width(&mut self, width: u16) {
        self.config.width = width;
    }

    /// Get current config
    pub fn config(&self) -> &RenderConfig {
        &self.config
    }

    /// Get mutable config
    pub fn config_mut(&mut self) -> &mut RenderConfig {
        &mut self.config
    }

    /// Register a placeholder (mutable entry) for in-place update tracking
    pub fn register_placeholder(&mut self, id: EntryId) {
        self.placeholders.register(id);
    }

    /// Track lines output (call after sending output to terminal)
    pub fn add_lines(&mut self, count: usize) {
        self.placeholders.add_lines(count);
    }

    /// Render only new entries since last render
    ///
    /// Returns the rendered string and number of lines
    pub fn render_incremental(&mut self, ledger: &Ledger) -> (String, usize) {
        let new_entries = ledger.since(self.last_rendered);

        if new_entries.is_empty() {
            return (String::new(), 0);
        }

        // Update last rendered
        self.last_rendered = ledger.last_id();

        let rendered = render_entries(new_entries, &self.config);
        let lines = count_lines(&rendered);

        (rendered, lines)
    }

    /// Force full re-render of all entries
    ///
    /// Returns the rendered string and number of lines
    pub fn render_full(&mut self, ledger: &Ledger) -> (String, usize) {
        self.last_rendered = ledger.last_id();
        self.placeholders = PlaceholderTracker::new(); // Reset placeholder tracking

        let rendered = render_entries(ledger.all(), &self.config);
        let lines = count_lines(&rendered);

        (rendered, lines)
    }

    /// Render a placeholder update (in-place replacement)
    ///
    /// Returns the escape sequence to go back, clear, and write new content,
    /// along with the number of lines in the new content.
    pub fn render_placeholder_update(
        &mut self,
        id: EntryId,
        ledger: &Ledger,
    ) -> Option<(String, usize)> {
        // Get the line offset for this placeholder
        let lines_back = self.placeholders.get_offset(id)?;

        // Get the updated entry
        let entry = ledger.get(id)?;

        // Render the new content
        let rendered = render_entry(entry, &self.config);
        let new_lines = count_lines(&rendered);

        // Build the update sequence
        let mut output = String::new();

        if lines_back > 0 {
            // Move cursor up to the placeholder line
            output.push_str(&styles::ctrl::move_up(lines_back as u16));
        }

        // Go to start of line and clear it
        output.push_str(styles::ctrl::CR);
        output.push_str(&styles::ctrl::clear_line());

        // Write the new content
        output.push_str(&rendered);

        // If the new content has more lines than the placeholder (1 line),
        // we need to handle the extra lines and redraw what was below
        // For now, we'll let the caller handle redrawing the prompt

        // Remove from tracking
        self.placeholders.remove(id);

        Some((output, new_lines))
    }

    /// Check if there are pending placeholders
    pub fn has_pending_placeholders(&self) -> bool {
        self.placeholders.has_pending()
    }

    /// Reset render state (for new session)
    pub fn reset(&mut self) {
        self.last_rendered = None;
        self.placeholders = PlaceholderTracker::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_buffer_incremental() {
        let mut ledger = Ledger::new(100);
        let mut display = DisplayBuffer::new(80);

        // Add first entry
        ledger.push(
            EntrySource::User("alice".into()),
            EntryContent::Chat("hello".into()),
        );

        let (output1, lines1) = display.render_incremental(&ledger);
        assert!(!output1.is_empty());
        assert!(lines1 > 0);

        // Add second entry
        ledger.push(
            EntrySource::User("bob".into()),
            EntryContent::Chat("hi".into()),
        );

        let (output2, _lines2) = display.render_incremental(&ledger);
        assert!(output2.contains("bob"));
        assert!(!output2.contains("alice")); // Only new entry
    }

    #[test]
    fn test_display_buffer_placeholder() {
        let mut ledger = Ledger::new(100);
        let mut display = DisplayBuffer::new(80);

        // Add a mutable placeholder
        let id = ledger.push_mutable(
            EntrySource::Model {
                name: "qwen".into(),
                is_streaming: false,
            },
            EntryContent::Status(StatusKind::Thinking),
        );

        let (output, _lines) = display.render_incremental(&ledger);
        assert!(output.contains("thinking"));

        // Register the placeholder
        display.register_placeholder(id);

        // Simulate some output happening
        display.add_lines(2);

        // Update the entry
        ledger.update(id, EntryContent::Chat("response".into()));

        // Render the update
        let update = display.render_placeholder_update(id, &ledger);
        assert!(update.is_some());

        let (update_output, _) = update.unwrap();
        assert!(update_output.contains("response"));
    }

    #[test]
    fn test_display_buffer_full_render() {
        let mut ledger = Ledger::new(100);
        let mut display = DisplayBuffer::new(80);

        ledger.push(EntrySource::System, EntryContent::Chat("one".into()));
        ledger.push(EntrySource::System, EntryContent::Chat("two".into()));

        // Do incremental first
        let _ = display.render_incremental(&ledger);

        // Add more
        ledger.push(EntrySource::System, EntryContent::Chat("three".into()));

        // Full render should include everything
        let (full, _) = display.render_full(&ledger);
        assert!(full.contains("one"));
        assert!(full.contains("two"));
        assert!(full.contains("three"));
    }
}
