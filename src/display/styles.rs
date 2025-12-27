//! Terminal styling via crossterm
//!
//! Provides semantic styling functions that return formatted strings.
//! Uses crossterm's styling but formats to String for SSH output.

use crossterm::style::{Attribute, Stylize};
use std::fmt::Write;

/// Format a username with cyan bold
pub fn username(name: &str) -> String {
    format!("{}", name.cyan().bold())
}

/// Format a model name with magenta
pub fn model_name(name: &str) -> String {
    format!("{}", name.magenta())
}

/// Format the prompt with yellow
pub fn prompt(text: &str) -> String {
    format!("{}", text.yellow())
}

/// Format status text as dim
pub fn dim(text: &str) -> String {
    format!("{}", text.attribute(Attribute::Dim))
}

/// Format error text as red
pub fn error(text: &str) -> String {
    format!("{}", text.red())
}

/// Format system message as gray
pub fn system(text: &str) -> String {
    format!("{}", text.dark_grey())
}

/// Format a timestamp as gray
pub fn timestamp(ts: &str) -> String {
    format!("[{}]", ts.dark_grey())
}

/// Create a horizontal line with optional label
pub fn separator(label: Option<&str>, width: u16) -> String {
    let line_char = "─";
    match label {
        Some(l) => {
            let label_len = l.chars().count();
            let side_len = (width.saturating_sub(label_len as u16 + 2) / 2) as usize;
            let left = line_char.repeat(side_len.max(3));
            let right = line_char.repeat(side_len.max(3));
            format!(
                "{}",
                format!("{} {} {}", left, l, right).dark_grey()
            )
        }
        None => format!("{}", line_char.repeat(width as usize).dark_grey()),
    }
}

/// Box drawing characters
pub struct BoxChars;

impl BoxChars {
    pub const TOP_LEFT: &'static str = "╭";
    pub const TOP_RIGHT: &'static str = "╮";
    pub const BOTTOM_LEFT: &'static str = "╰";
    pub const BOTTOM_RIGHT: &'static str = "╯";
    pub const HORIZONTAL: &'static str = "─";
    pub const VERTICAL: &'static str = "│";
}

/// Create a boxed header
pub fn boxed_header(title: &str, width: u16) -> String {
    let inner_width = (width.saturating_sub(4)) as usize;
    let title_len = title.chars().count();
    let padding = inner_width.saturating_sub(title_len);
    let left_pad = padding / 2;
    let right_pad = padding - left_pad;

    let horizontal = BoxChars::HORIZONTAL.repeat(inner_width);
    let padded_title = format!(
        "{}{}{}",
        " ".repeat(left_pad),
        title,
        " ".repeat(right_pad)
    );

    let mut result = String::new();
    // Use CRLF for SSH terminals
    let _ = write!(
        result,
        "{}{CRLF}",
        format!(
            "{}{}{}",
            BoxChars::TOP_LEFT,
            horizontal,
            BoxChars::TOP_RIGHT
        )
        .cyan()
        .bold(),
        CRLF = ctrl::CRLF
    );
    let _ = write!(
        result,
        "{}{CRLF}",
        format!(
            "{}{}{}",
            BoxChars::VERTICAL,
            padded_title,
            BoxChars::VERTICAL
        )
        .cyan()
        .bold(),
        CRLF = ctrl::CRLF
    );
    let _ = write!(
        result,
        "{}",
        format!(
            "{}{}{}",
            BoxChars::BOTTOM_LEFT,
            horizontal,
            BoxChars::BOTTOM_RIGHT
        )
        .cyan()
        .bold()
    );

    result
}

/// Terminal control sequences as strings (for SSH output)
pub mod ctrl {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_styled_output() {
        // Just verify these don't panic and produce non-empty output
        assert!(!username("alice").is_empty());
        assert!(!model_name("qwen").is_empty());
        assert!(!prompt("lobby>").is_empty());
        assert!(!dim("thinking...").is_empty());
        assert!(!error("oops").is_empty());
    }

    #[test]
    fn test_separator() {
        let sep = separator(Some("History"), 40);
        assert!(sep.contains("History"));
    }

    #[test]
    fn test_boxed_header() {
        let header = boxed_header("sshwarma", 40);
        assert!(header.contains("sshwarma"));
        assert!(header.contains(BoxChars::TOP_LEFT));
    }

    #[test]
    fn test_ctrl_sequences() {
        // Verify control sequences are non-empty ANSI
        assert!(!ctrl::move_up(1).is_empty());
        assert!(!ctrl::clear_line().is_empty());
        assert!(!ctrl::save_cursor().is_empty());
    }
}
