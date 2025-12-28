//! Braille spinner animation for active participants

/// Braille spinner frames (100ms cycle)
pub const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Get spinner character for the given frame
pub fn spinner_char(frame: u8) -> char {
    SPINNER_FRAMES[(frame as usize) % SPINNER_FRAMES.len()]
}
