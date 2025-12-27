//! Rendering ledger entries to terminal output
//!
//! Converts raw LedgerEntry data into formatted strings for display.

use super::ledger::{EntryContent, EntrySource, LedgerEntry, PresenceAction, StatusKind};
use super::styles::{self, ctrl};

/// Configuration for rendering
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// Terminal width
    pub width: u16,
    /// Show timestamps on entries
    pub show_timestamps: bool,
    /// Collapse consecutive blank lines
    pub collapse_blanks: bool,
    /// Maximum lines per entry before truncation
    pub max_lines_per_entry: usize,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            width: 80,
            show_timestamps: false,
            collapse_blanks: true,
            max_lines_per_entry: 50,
        }
    }
}

/// Render a single ledger entry to a string
pub fn render_entry(entry: &LedgerEntry, config: &RenderConfig) -> String {
    let mut output = String::new();

    // Optional timestamp prefix
    let ts_prefix = if config.show_timestamps {
        let ts = entry.timestamp.format("%H:%M");
        format!("{} ", styles::timestamp(&ts.to_string()))
    } else {
        String::new()
    };

    match &entry.content {
        EntryContent::Chat(text) => {
            let sender = format_sender(&entry.source);
            render_chat(&mut output, &ts_prefix, &sender, text, config);
        }

        EntryContent::CommandOutput(text) => {
            render_command_output(&mut output, text, config);
        }

        EntryContent::Status(kind) => {
            let text = match kind {
                StatusKind::Thinking => "thinking...",
                StatusKind::RunningTool => "running tool...",
                StatusKind::Connecting => "connecting...",
                StatusKind::Complete => "done",
            };
            output.push_str(&styles::dim(&format!("({})", text)));
        }

        EntryContent::RoomHeader { name, description } => {
            output.push_str(&styles::separator(None, config.width.min(50)));
            output.push_str(ctrl::CRLF);
            output.push_str(&format!("  {}  ", styles::username(name)));
            output.push_str(ctrl::CRLF);
            output.push_str(&styles::separator(None, config.width.min(50)));
            if let Some(desc) = description {
                output.push_str(ctrl::CRLF);
                output.push_str(&styles::dim(desc));
            }
        }

        EntryContent::Welcome { username } => {
            output.push_str(&styles::boxed_header("sshwarma", config.width.min(40)));
            output.push_str(ctrl::CRLF);
            output.push_str(ctrl::CRLF);
            output.push_str(&format!("Welcome, {}.", styles::username(username)));
            output.push_str(ctrl::CRLF);
            output.push_str(ctrl::CRLF);
            output.push_str("/rooms to list, /join <room> to enter");
        }

        EntryContent::HistorySeparator { label } => {
            output.push_str(&styles::separator(Some(label), config.width.min(50)));
        }

        EntryContent::Error(msg) => {
            output.push_str(&styles::error(msg));
        }

        EntryContent::Presence { user, action } => {
            let verb = match action {
                PresenceAction::Join => "joined",
                PresenceAction::Leave => "left",
            };
            output.push_str(&styles::system(&format!("* {} {}", user, verb)));
        }
    }

    output
}

/// Format a sender for display
fn format_sender(source: &EntrySource) -> String {
    match source {
        EntrySource::User(name) => styles::username(name),
        EntrySource::Model { name, .. } => styles::model_name(name),
        EntrySource::System => styles::system("*"),
        EntrySource::Command { command } => styles::dim(&format!("/{}", command)),
    }
}

/// Render a chat message with proper multi-line handling
fn render_chat(output: &mut String, ts_prefix: &str, sender: &str, text: &str, config: &RenderConfig) {
    let lines: Vec<&str> = text.lines().collect();

    if lines.is_empty() {
        output.push_str(ts_prefix);
        output.push_str(sender);
        output.push_str(": ");
        return;
    }

    // First line with sender
    output.push_str(ts_prefix);
    output.push_str(sender);
    output.push_str(": ");
    output.push_str(lines[0]);

    // Calculate indent for continuation lines
    // We need to account for ANSI codes in sender - use a fixed indent based on visible length
    let visible_sender_len = strip_ansi_len(sender);
    let indent = " ".repeat(visible_sender_len + 2); // +2 for ": "

    // Continuation lines
    let max_lines = config.max_lines_per_entry;
    for (i, line) in lines.iter().skip(1).take(max_lines.saturating_sub(1)).enumerate() {
        output.push_str(ctrl::CRLF);
        if i == 0 {
            // First continuation gets the timestamp indent too if present
            if !ts_prefix.is_empty() {
                output.push_str(&" ".repeat(6)); // "[HH:MM] " length
            }
        }
        output.push_str(&indent);
        output.push_str(line);
    }

    // Truncation notice
    if lines.len() > max_lines {
        output.push_str(ctrl::CRLF);
        output.push_str(&styles::dim(&format!("... ({} more lines)", lines.len() - max_lines)));
    }
}

/// Render command output with line limiting
fn render_command_output(output: &mut String, text: &str, config: &RenderConfig) {
    let lines: Vec<&str> = text.lines().collect();
    let max_lines = config.max_lines_per_entry;

    for (i, line) in lines.iter().take(max_lines).enumerate() {
        if i > 0 {
            output.push_str(ctrl::CRLF);
        }
        output.push_str(line);
    }

    if lines.len() > max_lines {
        output.push_str(ctrl::CRLF);
        output.push_str(&styles::dim(&format!("... ({} more lines)", lines.len() - max_lines)));
    }
}

/// Approximate visible length of a string (strips ANSI codes)
fn strip_ansi_len(s: &str) -> usize {
    // Simple heuristic: count chars outside of escape sequences
    let mut len = 0;
    let mut in_escape = false;

    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            len += 1;
        }
    }

    len
}

/// Render multiple entries with blank line collapsing
pub fn render_entries(entries: &[LedgerEntry], config: &RenderConfig) -> String {
    let mut output = String::new();

    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            output.push_str(ctrl::CRLF);
        }
        output.push_str(&render_entry(entry, config));
    }

    output
}

/// Count the number of terminal lines in rendered output
pub fn count_lines(rendered: &str) -> usize {
    if rendered.is_empty() {
        0
    } else {
        rendered.matches(ctrl::CRLF).count() + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::ledger::EntryId;
    use chrono::Utc;

    fn make_entry(source: EntrySource, content: EntryContent) -> LedgerEntry {
        LedgerEntry {
            id: EntryId(0),
            timestamp: Utc::now(),
            source,
            content,
            mutable: false,
            collapsible: true,
        }
    }

    #[test]
    fn test_render_chat() {
        let config = RenderConfig::default();
        let entry = make_entry(
            EntrySource::User("alice".into()),
            EntryContent::Chat("hello world".into()),
        );

        let rendered = render_entry(&entry, &config);
        assert!(rendered.contains("alice"));
        assert!(rendered.contains("hello world"));
    }

    #[test]
    fn test_render_multiline_chat() {
        let config = RenderConfig::default();
        let entry = make_entry(
            EntrySource::Model {
                name: "qwen".into(),
                is_streaming: false,
            },
            EntryContent::Chat("line1\nline2\nline3".into()),
        );

        let rendered = render_entry(&entry, &config);
        assert!(rendered.contains("qwen"));
        assert!(rendered.contains("line1"));
        assert!(rendered.contains("line2"));
        assert!(rendered.contains("line3"));
    }

    #[test]
    fn test_render_status() {
        let config = RenderConfig::default();
        let entry = make_entry(
            EntrySource::Model {
                name: "qwen".into(),
                is_streaming: false,
            },
            EntryContent::Status(StatusKind::Thinking),
        );

        let rendered = render_entry(&entry, &config);
        assert!(rendered.contains("thinking"));
    }

    #[test]
    fn test_render_with_timestamps() {
        let mut config = RenderConfig::default();
        config.show_timestamps = true;

        let entry = make_entry(
            EntrySource::User("bob".into()),
            EntryContent::Chat("test".into()),
        );

        let rendered = render_entry(&entry, &config);
        // Should have a timestamp
        assert!(rendered.contains("["));
        assert!(rendered.contains("]"));
    }

    #[test]
    fn test_count_lines() {
        assert_eq!(count_lines("single"), 1);
        assert_eq!(count_lines("one\r\ntwo"), 2);
        assert_eq!(count_lines("one\r\ntwo\r\nthree"), 3);
        assert_eq!(count_lines(""), 0);
    }

    #[test]
    fn test_strip_ansi_len() {
        assert_eq!(strip_ansi_len("hello"), 5);
        assert_eq!(strip_ansi_len("\x1b[31mred\x1b[0m"), 3);
        assert_eq!(strip_ansi_len("\x1b[1;36mbold cyan\x1b[0m"), 9);
    }
}
