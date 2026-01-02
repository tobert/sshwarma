//! Session state for tracking current buffer

/// Session-level state for buffer tracking
///
/// Tracks which buffer the user is viewing.
pub struct SessionState {
    /// Current room's buffer ID (None if in lobby)
    pub buffer_id: Option<String>,
}

impl SessionState {
    pub fn new() -> Self {
        Self { buffer_id: None }
    }

    /// Set current room buffer
    pub fn set_buffer(&mut self, buffer_id: Option<String>) {
        self.buffer_id = buffer_id;
    }

    /// Get current buffer ID
    pub fn buffer_id(&self) -> Option<&str> {
        self.buffer_id.as_deref()
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_state_new() {
        let state = SessionState::new();
        assert!(state.buffer_id.is_none());
    }

    #[test]
    fn test_set_buffer() {
        let mut state = SessionState::new();
        state.set_buffer(Some("buf_123".to_string()));
        assert_eq!(state.buffer_id(), Some("buf_123"));
        state.set_buffer(None);
        assert!(state.buffer_id().is_none());
    }
}
