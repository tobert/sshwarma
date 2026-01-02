//! Rich line editor for SSH REPL
//!
//! Provides readline-like editing over SSH:
//! - Cursor movement (arrow keys, home/end)
//! - Command history (up/down arrows)
//! - Kill/yank (Ctrl+K, Ctrl+U, Ctrl+W)
//! - Tab completion integration

use tui_input::Input;

use crate::ansi::TerminalEvent;

/// Rich line editor with history and cursor movement
pub struct LineEditor {
    /// Current input buffer with cursor position
    input: Input,
    /// Command history
    history: History,
    /// Position in history when navigating (None = editing new input)
    history_pos: Option<usize>,
    /// Saved input when navigating history
    saved_input: String,
    /// Terminal width for line wrapping calculations
    term_width: u16,
    /// Kill ring (last killed text for yanking)
    kill_ring: String,
}

impl LineEditor {
    pub fn new() -> Self {
        Self {
            input: Input::default(),
            history: History::new(500),
            history_pos: None,
            saved_input: String::new(),
            term_width: 80,
            kill_ring: String::new(),
        }
    }

    /// Set terminal width
    pub fn set_width(&mut self, width: u16) {
        self.term_width = width;
    }

    /// Get current input value
    pub fn value(&self) -> &str {
        self.input.value()
    }

    /// Get cursor position
    pub fn cursor(&self) -> usize {
        self.input.cursor()
    }

    /// Handle a terminal event, returns true if input changed
    pub fn handle_event(&mut self, event: TerminalEvent) -> EditorAction {
        match event {
            TerminalEvent::Char(c) => {
                self.cancel_history_nav();
                self.input.handle(tui_input::InputRequest::InsertChar(c));
                EditorAction::Redraw
            }
            TerminalEvent::Enter => {
                let line = self.submit();
                if let Some(ref text) = line {
                    EditorAction::Execute(text.clone())
                } else {
                    EditorAction::None
                }
            }
            TerminalEvent::Backspace => {
                self.cancel_history_nav();
                if self.input.cursor() > 0 {
                    self.input.handle(tui_input::InputRequest::DeletePrevChar);
                    EditorAction::Redraw
                } else {
                    EditorAction::None
                }
            }
            TerminalEvent::Delete => {
                self.cancel_history_nav();
                if self.input.cursor() < self.input.value().len() {
                    self.input.handle(tui_input::InputRequest::DeleteNextChar);
                    EditorAction::Redraw
                } else {
                    EditorAction::None
                }
            }
            TerminalEvent::Left => {
                if self.input.cursor() > 0 {
                    self.input.handle(tui_input::InputRequest::GoToPrevChar);
                    EditorAction::Redraw
                } else {
                    EditorAction::None
                }
            }
            TerminalEvent::Right => {
                if self.input.cursor() < self.input.value().len() {
                    self.input.handle(tui_input::InputRequest::GoToNextChar);
                    EditorAction::Redraw
                } else {
                    EditorAction::None
                }
            }
            TerminalEvent::Home | TerminalEvent::CtrlA => {
                self.input.handle(tui_input::InputRequest::GoToStart);
                EditorAction::Redraw
            }
            TerminalEvent::End | TerminalEvent::CtrlE => {
                self.input.handle(tui_input::InputRequest::GoToEnd);
                EditorAction::Redraw
            }
            TerminalEvent::Up => {
                self.history_prev();
                EditorAction::Redraw
            }
            TerminalEvent::Down => {
                self.history_next();
                EditorAction::Redraw
            }
            TerminalEvent::CtrlK => {
                self.kill_to_end();
                EditorAction::Redraw
            }
            TerminalEvent::CtrlU => {
                self.kill_to_start();
                EditorAction::Redraw
            }
            TerminalEvent::CtrlW => {
                self.kill_word_back();
                EditorAction::Redraw
            }
            TerminalEvent::CtrlC => {
                self.clear();
                EditorAction::Redraw
            }
            TerminalEvent::CtrlL => EditorAction::ClearScreen,
            TerminalEvent::CtrlD => {
                if self.input.value().is_empty() {
                    EditorAction::Quit
                } else {
                    EditorAction::None
                }
            }
            TerminalEvent::Tab => EditorAction::Tab,
            TerminalEvent::PageUp => EditorAction::PageUp,
            TerminalEvent::PageDown => EditorAction::PageDown,
            TerminalEvent::Unknown(_) => EditorAction::None,
        }
    }

    /// Submit current line, add to history, reset editor
    fn submit(&mut self) -> Option<String> {
        let line = self.input.value().to_string();
        if line.is_empty() {
            return None;
        }

        // Add to history (skip duplicates at end)
        if self.history.entries.last().map(|s| s.as_str()) != Some(&line) {
            self.history.push(line.clone());
        }

        // Reset state
        self.input.reset();
        self.history_pos = None;
        self.saved_input.clear();

        Some(line)
    }

    /// Navigate to previous history entry
    fn history_prev(&mut self) {
        if self.history.entries.is_empty() {
            return;
        }

        match self.history_pos {
            None => {
                // Starting navigation, save current input
                self.saved_input = self.input.value().to_string();
                self.history_pos = Some(self.history.entries.len() - 1);
            }
            Some(0) => {
                // Already at oldest entry
                return;
            }
            Some(pos) => {
                self.history_pos = Some(pos - 1);
            }
        }

        if let Some(pos) = self.history_pos {
            if let Some(entry) = self.history.entries.get(pos).cloned() {
                self.set_input(&entry);
            }
        }
    }

    /// Navigate to next history entry
    fn history_next(&mut self) {
        match self.history_pos {
            None => {
                // Not navigating history
            }
            Some(pos) if pos + 1 >= self.history.entries.len() => {
                // Return to saved input
                let saved = std::mem::take(&mut self.saved_input);
                self.set_input(&saved);
                self.history_pos = None;
            }
            Some(pos) => {
                self.history_pos = Some(pos + 1);
                if let Some(entry) = self.history.entries.get(pos + 1).cloned() {
                    self.set_input(&entry);
                }
            }
        }
    }

    /// Cancel history navigation (user started typing)
    fn cancel_history_nav(&mut self) {
        self.history_pos = None;
        self.saved_input.clear();
    }

    /// Set input to a new value (for history navigation)
    fn set_input(&mut self, value: &str) {
        self.input.reset();
        for c in value.chars() {
            self.input.handle(tui_input::InputRequest::InsertChar(c));
        }
    }

    /// Clear input
    fn clear(&mut self) {
        self.input.reset();
        self.history_pos = None;
        self.saved_input.clear();
    }

    /// Kill from cursor to end of line
    fn kill_to_end(&mut self) {
        let cursor = self.input.cursor();
        let value = self.input.value();
        if cursor < value.len() {
            self.kill_ring = value[cursor..].to_string();
            // Delete chars from cursor to end
            while self.input.cursor() < self.input.value().len() {
                self.input.handle(tui_input::InputRequest::DeleteNextChar);
            }
        }
    }

    /// Kill from start of line to cursor
    fn kill_to_start(&mut self) {
        let cursor = self.input.cursor();
        if cursor > 0 {
            let value = self.input.value();
            self.kill_ring = value[..cursor].to_string();
            // Delete chars from start to cursor
            for _ in 0..cursor {
                self.input.handle(tui_input::InputRequest::DeletePrevChar);
            }
        }
    }

    /// Kill word backward
    fn kill_word_back(&mut self) {
        let cursor = self.input.cursor();
        if cursor == 0 {
            return;
        }

        let value = self.input.value();
        let bytes = value.as_bytes();

        // Find start of word (skip trailing spaces, then find word start)
        let mut pos = cursor;

        // Skip trailing spaces
        while pos > 0 && bytes[pos - 1] == b' ' {
            pos -= 1;
        }

        // Find start of word
        while pos > 0 && bytes[pos - 1] != b' ' {
            pos -= 1;
        }

        // Kill from pos to cursor
        let killed = &value[pos..cursor];
        self.kill_ring = killed.to_string();

        for _ in 0..(cursor - pos) {
            self.input.handle(tui_input::InputRequest::DeletePrevChar);
        }
    }

    /// Insert completion text at cursor
    pub fn insert_completion(&mut self, text: &str) {
        for c in text.chars() {
            self.input.handle(tui_input::InputRequest::InsertChar(c));
        }
    }

    /// Replace partial text with completion
    pub fn replace_with_completion(&mut self, start: usize, text: &str) {
        // Delete from start to cursor
        let to_delete = self.input.cursor() - start;
        for _ in 0..to_delete {
            self.input.handle(tui_input::InputRequest::DeletePrevChar);
        }
        // Insert completion
        for c in text.chars() {
            self.input.handle(tui_input::InputRequest::InsertChar(c));
        }
    }

    /// Render the input line for display (ANSI output)
    pub fn render(&self, prompt: &str) -> String {
        let input = self.input.value();
        let cursor = self.input.cursor();

        // Calculate cursor column position (prompt length + input cursor)
        let prompt_len = prompt.chars().count();
        let cursor_col = prompt_len + 1 + cursor; // +1 for space after prompt

        // Clear line, draw prompt and input, position cursor
        format!(
            "\r\x1b[K\x1b[33m{}\x1b[0m {}\r\x1b[{}C",
            prompt, input, cursor_col
        )
    }

    /// Access history for persistence
    pub fn history(&self) -> &History {
        &self.history
    }

    /// Load history entries
    pub fn load_history(&mut self, entries: Vec<String>) {
        self.history.entries = entries;
    }
}

impl Default for LineEditor {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of handling an editor event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAction {
    /// No action needed
    None,
    /// Redraw the input line
    Redraw,
    /// Execute the given line
    Execute(String),
    /// Tab completion requested
    Tab,
    /// Clear screen requested
    ClearScreen,
    /// Quit (Ctrl+D on empty line)
    Quit,
    /// Scroll up one page
    PageUp,
    /// Scroll down one page
    PageDown,
}

/// Command history
pub struct History {
    entries: Vec<String>,
    max_size: usize,
}

impl History {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_size,
        }
    }

    pub fn push(&mut self, entry: String) {
        if entry.trim().is_empty() {
            return;
        }
        self.entries.push(entry);
        if self.entries.len() > self.max_size {
            self.entries.remove(0);
        }
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_input() {
        let mut editor = LineEditor::new();

        // Type "hello"
        for c in "hello".chars() {
            editor.handle_event(TerminalEvent::Char(c));
        }

        assert_eq!(editor.value(), "hello");
        assert_eq!(editor.cursor(), 5);
    }

    #[test]
    fn test_cursor_movement() {
        let mut editor = LineEditor::new();

        for c in "hello".chars() {
            editor.handle_event(TerminalEvent::Char(c));
        }

        // Move left twice
        editor.handle_event(TerminalEvent::Left);
        editor.handle_event(TerminalEvent::Left);
        assert_eq!(editor.cursor(), 3);

        // Insert character
        editor.handle_event(TerminalEvent::Char('X'));
        assert_eq!(editor.value(), "helXlo");
        assert_eq!(editor.cursor(), 4);

        // Move right
        editor.handle_event(TerminalEvent::Right);
        assert_eq!(editor.cursor(), 5);
    }

    #[test]
    fn test_home_end() {
        let mut editor = LineEditor::new();

        for c in "hello".chars() {
            editor.handle_event(TerminalEvent::Char(c));
        }

        editor.handle_event(TerminalEvent::Home);
        assert_eq!(editor.cursor(), 0);

        editor.handle_event(TerminalEvent::End);
        assert_eq!(editor.cursor(), 5);
    }

    #[test]
    fn test_history_navigation() {
        let mut editor = LineEditor::new();

        // Enter first command
        for c in "first".chars() {
            editor.handle_event(TerminalEvent::Char(c));
        }
        editor.handle_event(TerminalEvent::Enter);

        // Enter second command
        for c in "second".chars() {
            editor.handle_event(TerminalEvent::Char(c));
        }
        editor.handle_event(TerminalEvent::Enter);

        // Navigate back
        editor.handle_event(TerminalEvent::Up);
        assert_eq!(editor.value(), "second");

        editor.handle_event(TerminalEvent::Up);
        assert_eq!(editor.value(), "first");

        // Navigate forward
        editor.handle_event(TerminalEvent::Down);
        assert_eq!(editor.value(), "second");

        editor.handle_event(TerminalEvent::Down);
        assert_eq!(editor.value(), ""); // Back to empty new line
    }

    #[test]
    fn test_kill_word() {
        let mut editor = LineEditor::new();

        for c in "hello world".chars() {
            editor.handle_event(TerminalEvent::Char(c));
        }

        editor.handle_event(TerminalEvent::CtrlW);
        assert_eq!(editor.value(), "hello ");
    }
}
