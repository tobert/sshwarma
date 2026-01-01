//! Session state for tracking buffer rendering

use crate::db::rows::Row;
use crate::db::Database;
use crate::ui::render::render_rows;
use anyhow::Result;

/// Session-level state for buffer rendering
///
/// Tracks what has been rendered to enable incremental updates.
#[derive(Debug)]
pub struct SessionState {
    /// Current room's buffer ID (None if in lobby)
    pub buffer_id: Option<String>,
    /// Last rendered row ID (for incremental rendering)
    pub last_row_id: Option<String>,
    /// Terminal width
    pub width: usize,
    /// Lines rendered since last prompt
    pub lines_since_prompt: usize,
}

impl SessionState {
    pub fn new(width: usize) -> Self {
        Self {
            buffer_id: None,
            last_row_id: None,
            width,
            lines_since_prompt: 0,
        }
    }

    /// Set current room buffer
    pub fn set_buffer(&mut self, buffer_id: Option<String>) {
        self.buffer_id = buffer_id;
        self.last_row_id = None; // Reset on room change
        self.lines_since_prompt = 0;
    }

    /// Set terminal width
    pub fn set_width(&mut self, width: usize) {
        self.width = width;
    }

    /// Render all rows from buffer (full render)
    pub fn render_full(&mut self, db: &Database) -> Result<String> {
        let Some(ref buffer_id) = self.buffer_id else {
            return Ok(String::new());
        };

        let rows = db.list_buffer_rows(buffer_id)?;
        if let Some(last) = rows.last() {
            self.last_row_id = Some(last.id.clone());
        }

        let output = render_rows(&rows, self.width);
        self.lines_since_prompt = count_lines(&output);
        Ok(output)
    }

    /// Render only new rows since last render (incremental)
    pub fn render_incremental(&mut self, db: &Database) -> Result<String> {
        let Some(ref buffer_id) = self.buffer_id else {
            return Ok(String::new());
        };

        let rows = db.rows_since(buffer_id, self.last_row_id.as_deref())?;
        if rows.is_empty() {
            return Ok(String::new());
        }

        if let Some(last) = rows.last() {
            self.last_row_id = Some(last.id.clone());
        }

        let output = render_rows(&rows, self.width);
        self.lines_since_prompt += count_lines(&output);
        Ok(output)
    }

    /// Render a specific row (for placeholder updates)
    pub fn render_row(&self, row: &Row) -> String {
        render_rows(&[row.clone()], self.width)
    }

    /// Track additional lines output
    pub fn add_lines(&mut self, count: usize) {
        self.lines_since_prompt += count;
    }

    /// Get lines since last prompt (for cursor positioning)
    pub fn lines_back(&self) -> usize {
        self.lines_since_prompt
    }

    /// Reset line counter (after redraw)
    pub fn reset_lines(&mut self) {
        self.lines_since_prompt = 0;
    }
}

/// Count lines in output string
fn count_lines(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.matches("\r\n").count() + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_state_new() {
        let state = SessionState::new(80);
        assert_eq!(state.width, 80);
        assert!(state.buffer_id.is_none());
        assert!(state.last_row_id.is_none());
    }

    #[test]
    fn test_count_lines() {
        assert_eq!(count_lines(""), 0);
        assert_eq!(count_lines("hello"), 1);
        assert_eq!(count_lines("hello\r\nworld"), 2);
        assert_eq!(count_lines("a\r\nb\r\nc"), 3);
    }
}
