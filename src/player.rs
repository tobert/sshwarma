//! Player session state

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Per-connection player state
pub struct PlayerSession {
    pub session_id: String,
    pub username: String,
    pub connected_at: DateTime<Utc>,
    pub current_room: Option<String>,
}

impl PlayerSession {
    pub fn new(username: String) -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            username,
            connected_at: Utc::now(),
            current_room: None,
        }
    }
}
