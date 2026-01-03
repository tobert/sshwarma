//! ANSI escape sequence parser for terminal input
//!
//! Handles multi-byte escape sequences from SSH terminal input:
//! - Arrow keys (ESC [ A/B/C/D)
//! - Home/End (ESC [ H/F or ESC [ 1~ / ESC [ 4~)
//! - Delete (ESC [ 3~)
//! - Function keys, etc.

/// Parsed terminal input event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// Regular character input
    Char(char),
    /// Enter/Return key
    Enter,
    /// Backspace
    Backspace,
    /// Tab
    Tab,
    /// Escape key (bare ESC, not part of a sequence)
    Escape,
    /// Arrow keys
    Up,
    Down,
    Left,
    Right,
    /// Navigation
    Home,
    End,
    Delete,
    PageUp,
    PageDown,
    /// Control key combinations
    CtrlA,
    CtrlC,
    CtrlD,
    CtrlE,
    CtrlK,
    CtrlL,
    CtrlU,
    CtrlW,
    /// Unknown or unhandled
    Unknown(u8),
}

/// Parser state for multi-byte escape sequences
#[derive(Debug, Default)]
pub struct EscapeParser {
    state: ParseState,
    params: Vec<u8>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ParseState {
    #[default]
    Normal,
    /// Got ESC (0x1b)
    Escape,
    /// Got ESC [
    Csi,
}

impl EscapeParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a byte and return parsed event(s)
    pub fn feed(&mut self, byte: u8) -> Option<TerminalEvent> {
        match self.state {
            ParseState::Normal => self.handle_normal(byte),
            ParseState::Escape => self.handle_escape(byte),
            ParseState::Csi => self.handle_csi(byte),
        }
    }

    /// Flush pending state - call before processing new data packet
    /// Returns Escape event if we were waiting for a sequence that never came
    pub fn flush(&mut self) -> Option<TerminalEvent> {
        if self.state == ParseState::Escape {
            self.reset();
            Some(TerminalEvent::Escape)
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.state = ParseState::Normal;
        self.params.clear();
    }

    fn handle_normal(&mut self, byte: u8) -> Option<TerminalEvent> {
        match byte {
            // ESC - start escape sequence
            0x1b => {
                self.state = ParseState::Escape;
                None
            }
            // Control characters
            0x01 => Some(TerminalEvent::CtrlA), // Ctrl+A (home)
            0x03 => Some(TerminalEvent::CtrlC), // Ctrl+C (cancel)
            0x04 => Some(TerminalEvent::CtrlD), // Ctrl+D (EOF)
            0x05 => Some(TerminalEvent::CtrlE), // Ctrl+E (end)
            0x09 => Some(TerminalEvent::Tab),   // Tab
            0x0b => Some(TerminalEvent::CtrlK), // Ctrl+K (kill to end)
            0x0c => Some(TerminalEvent::CtrlL), // Ctrl+L (clear)
            0x0d => Some(TerminalEvent::Enter), // CR
            0x0a => Some(TerminalEvent::Enter), // LF
            0x15 => Some(TerminalEvent::CtrlU), // Ctrl+U (kill to start)
            0x17 => Some(TerminalEvent::CtrlW), // Ctrl+W (kill word)
            0x7f => Some(TerminalEvent::Backspace),
            0x08 => Some(TerminalEvent::Backspace),
            // Printable ASCII
            0x20..=0x7e => Some(TerminalEvent::Char(byte as char)),
            _ => Some(TerminalEvent::Unknown(byte)),
        }
    }

    fn handle_escape(&mut self, byte: u8) -> Option<TerminalEvent> {
        match byte {
            b'[' => {
                self.state = ParseState::Csi;
                self.params.clear();
                None
            }
            // ESC O sequences (some terminals send these for arrows)
            b'O' => {
                self.state = ParseState::Csi;
                self.params.clear();
                None
            }
            _ => {
                // Unknown escape sequence, reset and return ESC + byte
                self.reset();
                Some(TerminalEvent::Unknown(byte))
            }
        }
    }

    fn handle_csi(&mut self, byte: u8) -> Option<TerminalEvent> {
        match byte {
            // CSI parameter bytes (digits and semicolons)
            b'0'..=b'9' | b';' => {
                self.params.push(byte);
                None
            }
            // Arrow keys
            b'A' => {
                self.reset();
                Some(TerminalEvent::Up)
            }
            b'B' => {
                self.reset();
                Some(TerminalEvent::Down)
            }
            b'C' => {
                self.reset();
                Some(TerminalEvent::Right)
            }
            b'D' => {
                self.reset();
                Some(TerminalEvent::Left)
            }
            // Home/End
            b'H' => {
                self.reset();
                Some(TerminalEvent::Home)
            }
            b'F' => {
                self.reset();
                Some(TerminalEvent::End)
            }
            // Tilde sequences: ESC [ n ~
            b'~' => {
                let event = match self.params.as_slice() {
                    b"1" => TerminalEvent::Home,     // ESC [ 1 ~
                    b"3" => TerminalEvent::Delete,   // ESC [ 3 ~
                    b"4" => TerminalEvent::End,      // ESC [ 4 ~
                    b"5" => TerminalEvent::PageUp,   // ESC [ 5 ~
                    b"6" => TerminalEvent::PageDown, // ESC [ 6 ~
                    b"7" => TerminalEvent::Home,     // ESC [ 7 ~ (rxvt)
                    b"8" => TerminalEvent::End,      // ESC [ 8 ~ (rxvt)
                    _ => TerminalEvent::Unknown(b'~'),
                };
                self.reset();
                Some(event)
            }
            _ => {
                self.reset();
                Some(TerminalEvent::Unknown(byte))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_printable_chars() {
        let mut parser = EscapeParser::new();
        assert_eq!(parser.feed(b'a'), Some(TerminalEvent::Char('a')));
        assert_eq!(parser.feed(b'Z'), Some(TerminalEvent::Char('Z')));
        assert_eq!(parser.feed(b'5'), Some(TerminalEvent::Char('5')));
        assert_eq!(parser.feed(b' '), Some(TerminalEvent::Char(' ')));
    }

    #[test]
    fn test_control_chars() {
        let mut parser = EscapeParser::new();
        assert_eq!(parser.feed(0x0d), Some(TerminalEvent::Enter));
        assert_eq!(parser.feed(0x7f), Some(TerminalEvent::Backspace));
        assert_eq!(parser.feed(0x09), Some(TerminalEvent::Tab));
        assert_eq!(parser.feed(0x01), Some(TerminalEvent::CtrlA));
        assert_eq!(parser.feed(0x05), Some(TerminalEvent::CtrlE));
    }

    #[test]
    fn test_arrow_keys() {
        let mut parser = EscapeParser::new();

        // Up arrow: ESC [ A
        assert_eq!(parser.feed(0x1b), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'A'), Some(TerminalEvent::Up));

        // Down arrow: ESC [ B
        assert_eq!(parser.feed(0x1b), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'B'), Some(TerminalEvent::Down));

        // Right arrow: ESC [ C
        assert_eq!(parser.feed(0x1b), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'C'), Some(TerminalEvent::Right));

        // Left arrow: ESC [ D
        assert_eq!(parser.feed(0x1b), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'D'), Some(TerminalEvent::Left));
    }

    #[test]
    fn test_home_end() {
        let mut parser = EscapeParser::new();

        // Home: ESC [ H
        assert_eq!(parser.feed(0x1b), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'H'), Some(TerminalEvent::Home));

        // End: ESC [ F
        assert_eq!(parser.feed(0x1b), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'F'), Some(TerminalEvent::End));
    }

    #[test]
    fn test_delete() {
        let mut parser = EscapeParser::new();

        // Delete: ESC [ 3 ~
        assert_eq!(parser.feed(0x1b), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'3'), None);
        assert_eq!(parser.feed(b'~'), Some(TerminalEvent::Delete));
    }

    #[test]
    fn test_bare_escape() {
        let mut parser = EscapeParser::new();

        // Bare ESC: feed ESC, then flush (simulates no follow-up)
        assert_eq!(parser.feed(0x1b), None);
        assert_eq!(parser.flush(), Some(TerminalEvent::Escape));

        // Verify parser is reset
        assert_eq!(parser.flush(), None);
    }

    #[test]
    fn test_page_up_down() {
        let mut parser = EscapeParser::new();

        // PageUp: ESC [ 5 ~
        assert_eq!(parser.feed(0x1b), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'5'), None);
        assert_eq!(parser.feed(b'~'), Some(TerminalEvent::PageUp));

        // PageDown: ESC [ 6 ~
        assert_eq!(parser.feed(0x1b), None);
        assert_eq!(parser.feed(b'['), None);
        assert_eq!(parser.feed(b'6'), None);
        assert_eq!(parser.feed(b'~'), Some(TerminalEvent::PageDown));
    }
}
