//! Terminal control sequences for SSH output
//!
//! Provides ANSI escape sequences as strings for terminal manipulation.

use crossterm::cursor;
use crossterm::terminal::{Clear, ClearType};
use crossterm::Command;

/// Move cursor up N lines
pub fn move_up(n: u16) -> String {
    let mut buf = String::new();
    let _ = cursor::MoveUp(n).write_ansi(&mut buf);
    buf
}

/// Move cursor down N lines
pub fn move_down(n: u16) -> String {
    let mut buf = String::new();
    let _ = cursor::MoveDown(n).write_ansi(&mut buf);
    buf
}

/// Move cursor to column (0-indexed)
pub fn move_to_column(col: u16) -> String {
    let mut buf = String::new();
    let _ = cursor::MoveToColumn(col).write_ansi(&mut buf);
    buf
}

/// Clear current line
pub fn clear_line() -> String {
    let mut buf = String::new();
    let _ = Clear(ClearType::CurrentLine).write_ansi(&mut buf);
    buf
}

/// Clear from cursor to end of line
pub fn clear_to_eol() -> String {
    let mut buf = String::new();
    let _ = Clear(ClearType::UntilNewLine).write_ansi(&mut buf);
    buf
}

/// Clear screen
pub fn clear_screen() -> String {
    let mut buf = String::new();
    let _ = Clear(ClearType::All).write_ansi(&mut buf);
    buf
}

/// Save cursor position
pub fn save_cursor() -> String {
    let mut buf = String::new();
    let _ = cursor::SavePosition.write_ansi(&mut buf);
    buf
}

/// Restore cursor position
pub fn restore_cursor() -> String {
    let mut buf = String::new();
    let _ = cursor::RestorePosition.write_ansi(&mut buf);
    buf
}

/// Set scroll region (1-indexed, inclusive)
pub fn set_scroll_region(top: u16, bottom: u16) -> String {
    format!("\x1b[{};{}r", top, bottom)
}

/// Reset scroll region to full screen
pub fn reset_scroll_region() -> String {
    "\x1b[r".to_string()
}

/// Move cursor to absolute position (1-indexed)
pub fn move_to(row: u16, col: u16) -> String {
    format!("\x1b[{};{}H", row, col)
}

/// Scroll up N lines within scroll region
pub fn scroll_up(n: u16) -> String {
    format!("\x1b[{}S", n)
}

/// Carriage return + newline (for SSH terminals)
pub const CRLF: &str = "\r\n";

/// Carriage return only
pub const CR: &str = "\r";

/// Backspace
pub const BACKSPACE: &str = "\x08";

/// Cursor left
pub const LEFT: &str = "\x1b[D";

/// Cursor right
pub const RIGHT: &str = "\x1b[C";

/// HUD dimensions (8 lines: 7 for display, 1 for input prompt)
pub const HUD_HEIGHT: u16 = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ctrl_sequences() {
        assert!(!move_up(1).is_empty());
        assert!(!clear_line().is_empty());
        assert!(!save_cursor().is_empty());
    }
}
