//! Input handling for Lua-driven UI
//!
//! Provides key event routing, input buffer management, and completion
//! that can be customized via Lua scripts.

use mlua::prelude::*;
use std::sync::{Arc, Mutex};

/// Key event types that can be routed to Lua
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
            .map(|i| i + 1)
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
        if !text.trim().is_empty()
            && self.history.last().map(|s| s.as_str()) != Some(&text) {
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
                // Include the trigger character (/ or @) but not whitespace
                if before.as_bytes().get(i).map(|&b| b == b'/' || b == b'@') == Some(true) {
                    i
                } else {
                    i + 1
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

/// A completion candidate
#[derive(Debug, Clone)]
pub struct CompletionItem {
    /// Text to insert
    pub text: String,
    /// Display label
    pub label: String,
    /// Optional description
    pub description: Option<String>,
    /// Match score (higher = better)
    pub score: u32,
}

/// Completion state
#[derive(Debug, Clone, Default)]
pub struct CompletionState {
    /// Available completions
    pub items: Vec<CompletionItem>,
    /// Currently selected index
    pub selected: usize,
    /// Start position in input where completion applies
    pub start: usize,
    /// End position in input where completion applies
    pub end: usize,
    /// Whether completion menu is visible
    pub visible: bool,
}

impl CompletionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update with new completions
    pub fn update(&mut self, items: Vec<CompletionItem>, start: usize, end: usize) {
        self.items = items;
        self.selected = 0;
        self.start = start;
        self.end = end;
        self.visible = !self.items.is_empty();
    }

    /// Select next completion
    pub fn next(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1) % self.items.len();
        }
    }

    /// Select previous completion
    pub fn prev(&mut self) {
        if !self.items.is_empty() {
            self.selected = self.selected.checked_sub(1).unwrap_or(self.items.len() - 1);
        }
    }

    /// Get currently selected item
    pub fn current(&self) -> Option<&CompletionItem> {
        self.items.get(self.selected)
    }

    /// Clear completions
    pub fn clear(&mut self) {
        self.items.clear();
        self.selected = 0;
        self.visible = false;
    }
}

/// Lua userdata for input buffer
#[derive(Clone)]
pub struct LuaInputBuffer {
    buffer: Arc<Mutex<InputBuffer>>,
    completion: Arc<Mutex<CompletionState>>,
}

impl LuaInputBuffer {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(InputBuffer::new())),
            completion: Arc::new(Mutex::new(CompletionState::new())),
        }
    }

    pub fn buffer(&self) -> Arc<Mutex<InputBuffer>> {
        self.buffer.clone()
    }

    pub fn completion(&self) -> Arc<Mutex<CompletionState>> {
        self.completion.clone()
    }
}

impl Default for LuaInputBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl LuaUserData for LuaInputBuffer {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // Properties via index
        methods.add_meta_method(mlua::MetaMethod::Index, |_lua, this, key: String| {
            let buf = this.buffer.lock().unwrap();
            match key.as_str() {
                "text" => Ok(LuaValue::String(_lua.create_string(&buf.text)?)),
                "cursor" => Ok(LuaValue::Integer(buf.cursor as i32)),
                "len" => Ok(LuaValue::Integer(buf.text.len() as i32)),
                _ => Ok(LuaValue::Nil),
            }
        });

        // input:insert(text)
        methods.add_method("insert", |_lua, this, text: String| {
            let mut buf = this.buffer.lock().unwrap();
            buf.insert_str(&text);
            Ok(())
        });

        // input:backspace() -> bool
        methods.add_method("backspace", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            Ok(buf.backspace())
        });

        // input:delete() -> bool
        methods.add_method("delete", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            Ok(buf.delete())
        });

        // input:left() -> bool
        methods.add_method("left", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            Ok(buf.move_left())
        });

        // input:right() -> bool
        methods.add_method("right", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            Ok(buf.move_right())
        });

        // input:home()
        methods.add_method("home", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            buf.move_home();
            Ok(())
        });

        // input:end_()
        methods.add_method("end_", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            buf.move_end();
            Ok(())
        });

        // input:clear()
        methods.add_method("clear", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            buf.clear();
            Ok(())
        });

        // input:set(text)
        methods.add_method("set", |_lua, this, text: String| {
            let mut buf = this.buffer.lock().unwrap();
            buf.set(&text);
            Ok(())
        });

        // input:submit() -> string
        methods.add_method("submit", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            Ok(buf.submit())
        });

        // input:history_prev() -> bool
        methods.add_method("history_prev", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            Ok(buf.history_prev())
        });

        // input:history_next() -> bool
        methods.add_method("history_next", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            Ok(buf.history_next())
        });

        // input:kill_to_end() -> string
        methods.add_method("kill_to_end", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            Ok(buf.kill_to_end())
        });

        // input:kill_to_start() -> string
        methods.add_method("kill_to_start", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            Ok(buf.kill_to_start())
        });

        // input:kill_word() -> string
        methods.add_method("kill_word", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            Ok(buf.kill_word())
        });

        // input:word_at_cursor() -> word, start, end
        methods.add_method("word_at_cursor", |_lua, this, ()| {
            let buf = this.buffer.lock().unwrap();
            let (word, start, end) = buf.word_at_cursor();
            Ok((word.to_string(), start as i32, end as i32))
        });

        // Completion methods

        // input:complete_next()
        methods.add_method("complete_next", |_lua, this, ()| {
            let mut comp = this.completion.lock().unwrap();
            comp.next();
            Ok(())
        });

        // input:complete_prev()
        methods.add_method("complete_prev", |_lua, this, ()| {
            let mut comp = this.completion.lock().unwrap();
            comp.prev();
            Ok(())
        });

        // input:complete_accept() -> bool (returns true if completion was applied)
        methods.add_method("complete_accept", |_lua, this, ()| {
            let mut buf = this.buffer.lock().unwrap();
            let mut comp = this.completion.lock().unwrap();

            if let Some(item) = comp.current() {
                // Replace the word being completed
                let new_text = format!(
                    "{}{}{}",
                    &buf.text[..comp.start],
                    item.text,
                    &buf.text[comp.end..]
                );
                buf.text = new_text;
                buf.cursor = comp.start + item.text.len();
                comp.clear();
                Ok(true)
            } else {
                Ok(false)
            }
        });

        // input:complete_cancel()
        methods.add_method("complete_cancel", |_lua, this, ()| {
            let mut comp = this.completion.lock().unwrap();
            comp.clear();
            Ok(())
        });

        // input:completions() -> table of {text, label, description?, score}
        methods.add_method("completions", |lua, this, ()| {
            let comp = this.completion.lock().unwrap();
            let tbl = lua.create_table()?;

            for (i, item) in comp.items.iter().enumerate() {
                let item_tbl = lua.create_table()?;
                item_tbl.set("text", item.text.as_str())?;
                item_tbl.set("label", item.label.as_str())?;
                if let Some(desc) = &item.description {
                    item_tbl.set("description", desc.as_str())?;
                }
                item_tbl.set("score", item.score)?;
                item_tbl.set("selected", i == comp.selected)?;
                tbl.set(i + 1, item_tbl)?;
            }

            Ok(tbl)
        });

        // input:completion_visible() -> bool
        methods.add_method("completion_visible", |_lua, this, ()| {
            let comp = this.completion.lock().unwrap();
            Ok(comp.visible)
        });
    }
}

/// Register input functions in Lua
pub fn register_input_functions(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();
    let sshwarma: LuaTable = globals.get("sshwarma")?;

    // sshwarma.input_buffer() -> LuaInputBuffer
    sshwarma.set(
        "input_buffer",
        lua.create_function(|_lua, ()| Ok(LuaInputBuffer::new()))?,
    )?;

    // Create completer registry as a Lua table
    let completers = lua.create_table()?;
    sshwarma.set("_completers", completers)?;

    // sshwarma.register_completer(name, function)
    sshwarma.set(
        "register_completer",
        lua.create_function(|lua, (name, func): (String, LuaFunction)| {
            let sshwarma: LuaTable = lua.globals().get("sshwarma")?;
            let completers: LuaTable = sshwarma.get("_completers")?;
            completers.set(name, func)?;
            Ok(())
        })?,
    )?;

    // sshwarma.unregister_completer(name)
    sshwarma.set(
        "unregister_completer",
        lua.create_function(|lua, name: String| {
            let sshwarma: LuaTable = lua.globals().get("sshwarma")?;
            let completers: LuaTable = sshwarma.get("_completers")?;
            completers.set(name, LuaValue::Nil)?;
            Ok(())
        })?,
    )?;

    // sshwarma.complete(text, cursor) -> [{text, label, description?, score}]
    sshwarma.set(
        "complete",
        lua.create_function(|lua, (text, cursor): (String, i32)| {
            let sshwarma: LuaTable = lua.globals().get("sshwarma")?;
            let completers: LuaTable = sshwarma.get("_completers")?;
            let results = lua.create_table()?;
            let mut idx = 1;

            // Call each registered completer
            for (_, func) in completers.pairs::<String, LuaFunction>().flatten() {
                let result: LuaValue = func.call((text.as_str(), cursor))?;
                if let LuaValue::Table(items) = result {
                    for (_, item) in items.pairs::<i32, LuaTable>().flatten() {
                        results.set(idx, item)?;
                        idx += 1;
                    }
                }
            }

            Ok(results)
        })?,
    )?;

    Ok(())
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

    #[test]
    fn test_completion_state() {
        let mut comp = CompletionState::new();

        let items = vec![
            CompletionItem {
                text: "first".to_string(),
                label: "First".to_string(),
                description: None,
                score: 100,
            },
            CompletionItem {
                text: "second".to_string(),
                label: "Second".to_string(),
                description: Some("Description".to_string()),
                score: 90,
            },
        ];

        comp.update(items, 0, 5);
        assert!(comp.visible);
        assert_eq!(comp.selected, 0);
        assert_eq!(comp.current().unwrap().text, "first");

        comp.next();
        assert_eq!(comp.selected, 1);
        assert_eq!(comp.current().unwrap().text, "second");

        comp.next(); // Wraps around
        assert_eq!(comp.selected, 0);

        comp.prev(); // Wraps to end
        assert_eq!(comp.selected, 1);
    }

    #[test]
    fn test_lua_input_integration() -> anyhow::Result<()> {
        let lua = Lua::new();

        let sshwarma = lua.create_table()?;
        lua.globals().set("sshwarma", sshwarma)?;

        register_input_functions(&lua)?;

        lua.load(
            r#"
            local input = sshwarma.input_buffer()

            input:insert("Hello")
            assert(input.text == "Hello", "text should be Hello")
            assert(input.cursor == 5, "cursor should be 5")

            input:left()
            assert(input.cursor == 4, "cursor should be 4 after left")

            input:insert("!")
            assert(input.text == "Hell!o", "text should be Hell!o")

            input:home()
            assert(input.cursor == 0, "cursor should be 0 after home")

            input:end_()
            assert(input.cursor == 6, "cursor should be 6 after end")

            local submitted = input:submit()
            assert(submitted == "Hell!o", "submitted should be Hell!o")
            assert(input.text == "", "text should be empty after submit")
        "#,
        )
        .exec()?;

        Ok(())
    }

    #[test]
    fn test_lua_completer_registration() -> anyhow::Result<()> {
        let lua = Lua::new();

        let sshwarma = lua.create_table()?;
        lua.globals().set("sshwarma", sshwarma)?;

        register_input_functions(&lua)?;

        lua.load(
            r#"
            -- Register a simple completer
            sshwarma.register_completer("test", function(text, cursor)
                if text:sub(1, 1) == "/" then
                    return {
                        { text = "/help", label = "Help", score = 100 },
                        { text = "/quit", label = "Quit", score = 90 },
                    }
                end
                return {}
            end)

            -- Unregister it
            sshwarma.unregister_completer("test")
        "#,
        )
        .exec()?;

        Ok(())
    }
}
