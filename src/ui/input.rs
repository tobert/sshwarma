//! Input handling for terminal UI
//!
//! Provides key event types and input buffer management.

/// Key event types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyEvent {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Escape,
    Ctrl(char),
    Alt(char),
}

impl KeyEvent {
    /// Convert to Lua-friendly string representation
    pub fn to_lua_string(&self) -> String {
        match self {
            KeyEvent::Char(c) => format!("char:{}", c),
            KeyEvent::Enter => "enter".to_string(),
            KeyEvent::Tab => "tab".to_string(),
            KeyEvent::Backspace => "backspace".to_string(),
            KeyEvent::Delete => "delete".to_string(),
            KeyEvent::Left => "left".to_string(),
            KeyEvent::Right => "right".to_string(),
            KeyEvent::Up => "up".to_string(),
            KeyEvent::Down => "down".to_string(),
            KeyEvent::Home => "home".to_string(),
            KeyEvent::End => "end".to_string(),
            KeyEvent::PageUp => "pageup".to_string(),
            KeyEvent::PageDown => "pagedown".to_string(),
            KeyEvent::Escape => "escape".to_string(),
            KeyEvent::Ctrl(c) => format!("ctrl:{}", c),
            KeyEvent::Alt(c) => format!("alt:{}", c),
        }
    }

    /// Parse from Lua string representation
    pub fn from_lua_string(s: &str) -> Option<Self> {
        if let Some(c) = s.strip_prefix("char:") {
            return c.chars().next().map(KeyEvent::Char);
        }
        if let Some(c) = s.strip_prefix("ctrl:") {
            return c.chars().next().map(KeyEvent::Ctrl);
        }
        if let Some(c) = s.strip_prefix("alt:") {
            return c.chars().next().map(KeyEvent::Alt);
        }

        Some(match s {
            "enter" => KeyEvent::Enter,
            "tab" => KeyEvent::Tab,
            "backspace" => KeyEvent::Backspace,
            "delete" => KeyEvent::Delete,
            "left" => KeyEvent::Left,
            "right" => KeyEvent::Right,
            "up" => KeyEvent::Up,
            "down" => KeyEvent::Down,
            "home" => KeyEvent::Home,
            "end" => KeyEvent::End,
            "pageup" => KeyEvent::PageUp,
            "pagedown" => KeyEvent::PageDown,
            "escape" => KeyEvent::Escape,
            _ => return None,
        })
    }
}

/// Input buffer with cursor and history
#[derive(Debug, Clone)]
pub struct InputBuffer {
    /// Current text content
    pub text: String,
    /// Cursor position (byte offset)
    pub cursor: usize,
    /// Command history
    history: Vec<String>,
    /// Current position in history (None = new input)
    history_pos: Option<usize>,
    /// Saved input when navigating history
    saved_input: String,
    /// Maximum history size
    max_history: usize,
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl InputBuffer {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_pos: None,
            saved_input: String::new(),
            max_history: 500,
        }
    }

    /// Insert a character at cursor
    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Insert a string at cursor
    pub fn insert_str(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Delete character before cursor
    pub fn backspace(&mut self) -> bool {
        if self.cursor > 0 {
            // Find the previous character boundary
            let prev = self.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.remove(prev);
            self.cursor = prev;
            true
        } else {
            false
        }
    }

    /// Delete character at cursor
    pub fn delete(&mut self) -> bool {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
            true
        } else {
            false
        }
    }

    /// Move cursor left
    pub fn move_left(&mut self) -> bool {
        if self.cursor > 0 {
            let prev = self.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor = prev;
            true
        } else {
            false
        }
    }

    /// Move cursor right
    pub fn move_right(&mut self) -> bool {
        if self.cursor < self.text.len() {
            let next = self.text[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.text.len());
            self.cursor = next;
            true
        } else {
            false
        }
    }

    /// Move cursor to start
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end
    pub fn move_end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Delete from cursor to end of line
    pub fn kill_to_end(&mut self) -> String {
        let killed = self.text[self.cursor..].to_string();
        self.text.truncate(self.cursor);
        killed
    }

    /// Delete from start to cursor
    pub fn kill_to_start(&mut self) -> String {
        let killed = self.text[..self.cursor].to_string();
        self.text = self.text[self.cursor..].to_string();
        self.cursor = 0;
        killed
    }

    /// Delete previous word
    pub fn kill_word(&mut self) -> String {
        if self.cursor == 0 {
            return String::new();
        }

        // Find start of word (skip trailing whitespace, then non-whitespace)
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        let word_start = trimmed
            .rfind(char::is_whitespace)
            .map(|i| {
                // Skip past the whitespace char (which may be multi-byte)
                let ws_char = trimmed[i..].chars().next().unwrap();
                i + ws_char.len_utf8()
            })
            .unwrap_or(0);

        let killed = self.text[word_start..self.cursor].to_string();
        self.text = format!("{}{}", &self.text[..word_start], &self.text[self.cursor..]);
        self.cursor = word_start;
        killed
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Set text and move cursor to end
    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.len();
    }

    /// Submit current input (adds to history, clears buffer)
    pub fn submit(&mut self) -> String {
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.history_pos = None;

        // Add to history if non-empty and different from last
        if !text.trim().is_empty() && self.history.last().map(|s| s.as_str()) != Some(&text) {
            self.history.push(text.clone());
            if self.history.len() > self.max_history {
                self.history.remove(0);
            }
        }

        text
    }

    /// Navigate to previous history entry
    pub fn history_prev(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }

        match self.history_pos {
            None => {
                // Save current input and go to last history entry
                self.saved_input = self.text.clone();
                self.history_pos = Some(self.history.len() - 1);
            }
            Some(pos) if pos > 0 => {
                self.history_pos = Some(pos - 1);
            }
            _ => return false,
        }

        if let Some(pos) = self.history_pos {
            self.text = self.history[pos].clone();
            self.cursor = self.text.len();
        }
        true
    }

    /// Navigate to next history entry
    pub fn history_next(&mut self) -> bool {
        match self.history_pos {
            Some(pos) if pos < self.history.len() - 1 => {
                self.history_pos = Some(pos + 1);
                self.text = self.history[pos + 1].clone();
                self.cursor = self.text.len();
                true
            }
            Some(_) => {
                // Restore saved input
                self.history_pos = None;
                self.text = std::mem::take(&mut self.saved_input);
                self.cursor = self.text.len();
                true
            }
            None => false,
        }
    }

    /// Get word at/before cursor for completion
    pub fn word_at_cursor(&self) -> (&str, usize, usize) {
        let before = &self.text[..self.cursor];

        // Find word start
        let start = before
            .rfind(|c: char| c.is_whitespace() || c == '/' || c == '@')
            .map(|i| {
                let found_char = before[i..].chars().next().unwrap();
                // Include the trigger character (/ or @) but not whitespace
                if found_char == '/' || found_char == '@' {
                    i
                } else {
                    // Skip past the whitespace (which may be multi-byte)
                    i + found_char.len_utf8()
                }
            })
            .unwrap_or(0);

        // Find word end (could extend past cursor)
        let end = self.text[self.cursor..]
            .find(char::is_whitespace)
            .map(|i| self.cursor + i)
            .unwrap_or(self.text.len());

        (&self.text[start..end], start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_event_roundtrip() {
        let events = vec![
            KeyEvent::Char('a'),
            KeyEvent::Enter,
            KeyEvent::Tab,
            KeyEvent::Ctrl('c'),
            KeyEvent::Alt('x'),
        ];

        for event in events {
            let s = event.to_lua_string();
            let parsed = KeyEvent::from_lua_string(&s).unwrap();
            assert_eq!(event, parsed);
        }
    }

    #[test]
    fn test_input_buffer_basic() {
        let mut buf = InputBuffer::new();

        buf.insert('H');
        buf.insert('i');
        assert_eq!(buf.text, "Hi");
        assert_eq!(buf.cursor, 2);

        buf.move_left();
        assert_eq!(buf.cursor, 1);

        buf.insert('!');
        assert_eq!(buf.text, "H!i");
        assert_eq!(buf.cursor, 2);
    }

    #[test]
    fn test_input_buffer_backspace() {
        let mut buf = InputBuffer::new();
        buf.set("Hello");

        buf.backspace();
        assert_eq!(buf.text, "Hell");

        buf.move_home();
        assert!(!buf.backspace()); // Can't backspace at start
        assert_eq!(buf.text, "Hell");
    }

    #[test]
    fn test_input_buffer_kill() {
        let mut buf = InputBuffer::new();
        buf.set("Hello World");
        buf.cursor = 6; // After "Hello "

        let killed = buf.kill_to_end();
        assert_eq!(killed, "World");
        assert_eq!(buf.text, "Hello ");

        buf.set("Hello World");
        buf.cursor = 6;
        let killed = buf.kill_to_start();
        assert_eq!(killed, "Hello ");
        assert_eq!(buf.text, "World");
    }

    #[test]
    fn test_input_buffer_kill_word() {
        let mut buf = InputBuffer::new();
        buf.set("Hello World Test");
        buf.cursor = 11; // After "Hello World"

        let killed = buf.kill_word();
        assert_eq!(killed, "World");
        assert_eq!(buf.text, "Hello  Test");
    }

    #[test]
    fn test_input_buffer_history() {
        let mut buf = InputBuffer::new();

        buf.set("first");
        buf.submit();
        buf.set("second");
        buf.submit();
        buf.set("third");
        buf.submit();

        // Navigate back
        buf.set("current");
        assert!(buf.history_prev());
        assert_eq!(buf.text, "third");

        assert!(buf.history_prev());
        assert_eq!(buf.text, "second");

        // Navigate forward
        assert!(buf.history_next());
        assert_eq!(buf.text, "third");

        assert!(buf.history_next());
        assert_eq!(buf.text, "current"); // Restored saved input
    }

    #[test]
    fn test_word_at_cursor() {
        let mut buf = InputBuffer::new();

        buf.set("/join studio");
        buf.cursor = 6; // After "/join "
        let (word, start, end) = buf.word_at_cursor();
        assert_eq!(word, "studio");
        assert_eq!(start, 6);
        assert_eq!(end, 12);

        buf.set("@model hello");
        buf.cursor = 4; // In "@mod"
        let (word, start, end) = buf.word_at_cursor();
        assert_eq!(word, "@model");
        assert_eq!(start, 0);
        assert_eq!(end, 6);
    }

    // ==========================================================================
    // Unicode edge case tests
    // ==========================================================================

    #[test]
    fn test_unicode_navigation_emoji() {
        let mut buf = InputBuffer::new();
        // 🎵 is 4 bytes (U+1F3B5)
        buf.set("a🎵b");
        assert_eq!(buf.text.len(), 6); // 1 + 4 + 1 bytes
        assert_eq!(buf.cursor, 6); // at end

        // Move left through 'b'
        buf.move_left();
        assert_eq!(buf.cursor, 5); // before 'b', after emoji

        // Move left through emoji (should skip all 4 bytes)
        buf.move_left();
        assert_eq!(buf.cursor, 1); // before emoji, after 'a'

        // Move left through 'a'
        buf.move_left();
        assert_eq!(buf.cursor, 0);

        // Move right through 'a'
        buf.move_right();
        assert_eq!(buf.cursor, 1);

        // Move right through emoji
        buf.move_right();
        assert_eq!(buf.cursor, 5); // after emoji
    }

    #[test]
    fn test_unicode_navigation_cjk() {
        let mut buf = InputBuffer::new();
        // 日本語 - each CJK char is 3 bytes
        buf.set("日本語");
        assert_eq!(buf.text.len(), 9); // 3 * 3 bytes
        assert_eq!(buf.cursor, 9);

        buf.move_left();
        assert_eq!(buf.cursor, 6); // before 語

        buf.move_left();
        assert_eq!(buf.cursor, 3); // before 本

        buf.move_left();
        assert_eq!(buf.cursor, 0); // before 日
    }

    #[test]
    fn test_unicode_backspace_emoji() {
        let mut buf = InputBuffer::new();
        buf.set("hello🎵world");

        // Position after emoji
        buf.cursor = 9; // "hello" (5) + emoji (4)

        // Backspace should delete entire emoji
        buf.backspace();
        assert_eq!(buf.text, "helloworld");
        assert_eq!(buf.cursor, 5);
    }

    #[test]
    fn test_unicode_insert_at_boundary() {
        let mut buf = InputBuffer::new();
        buf.set("日本");
        buf.cursor = 3; // between 日 and 本

        buf.insert('X');
        assert_eq!(buf.text, "日X本");
        assert_eq!(buf.cursor, 4); // after 'X'
    }

    #[test]
    fn test_unicode_delete_multibyte() {
        let mut buf = InputBuffer::new();
        buf.set("a🎵b");
        buf.cursor = 1; // before emoji

        // Delete should remove entire emoji
        buf.delete();
        assert_eq!(buf.text, "ab");
    }

    #[test]
    fn test_kill_word_with_multibyte_whitespace() {
        let mut buf = InputBuffer::new();
        // U+00A0 is non-breaking space (2 bytes in UTF-8: 0xC2 0xA0)
        buf.set("hello\u{00A0}world");
        buf.cursor = buf.text.len(); // at end

        // This should kill "world" and leave "hello\u{00A0}"
        let killed = buf.kill_word();
        assert_eq!(killed, "world");
        assert_eq!(buf.text, "hello\u{00A0}");
    }

    #[test]
    fn test_kill_word_with_ideographic_space() {
        let mut buf = InputBuffer::new();
        // U+3000 is ideographic space (3 bytes in UTF-8)
        buf.set("hello\u{3000}world");
        buf.cursor = buf.text.len();

        let killed = buf.kill_word();
        assert_eq!(killed, "world");
        assert_eq!(buf.text, "hello\u{3000}");
    }

    #[test]
    fn test_word_at_cursor_multibyte_whitespace() {
        let mut buf = InputBuffer::new();
        // U+00A0 non-breaking space between words
        buf.set("hello\u{00A0}world");
        buf.cursor = buf.text.len(); // at end, in "world"

        let (word, start, end) = buf.word_at_cursor();
        assert_eq!(word, "world");
        // start should be after the NBSP (byte 7, not 6)
        assert_eq!(start, 7); // "hello" (5) + NBSP (2)
        assert_eq!(end, 12);
    }

    #[test]
    fn test_word_at_cursor_ideographic_space() {
        let mut buf = InputBuffer::new();
        // U+3000 ideographic space (3 bytes)
        buf.set("日本\u{3000}語");
        buf.cursor = buf.text.len(); // at end, in "語"

        let (word, start, end) = buf.word_at_cursor();
        assert_eq!(word, "語");
        // start should be after ideographic space
        assert_eq!(start, 9); // "日本" (6) + space (3)
        assert_eq!(end, 12);
    }

    #[test]
    fn test_mixed_ascii_unicode_navigation() {
        let mut buf = InputBuffer::new();
        buf.set("café"); // 'é' is 2 bytes (U+00E9)
        assert_eq!(buf.text.len(), 5); // c(1) + a(1) + f(1) + é(2)

        buf.move_left(); // from end
        assert_eq!(buf.cursor, 3); // before 'é'

        buf.move_left();
        assert_eq!(buf.cursor, 2); // before 'f'
    }
}
