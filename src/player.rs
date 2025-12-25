//! Player session state

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Per-connection player state
pub struct PlayerSession {
    pub session_id: String,
    pub username: String,
    pub connected_at: DateTime<Utc>,
    pub current_room: Option<String>,
    pub inventory: Vec<String>,
    pub stats: PlayerStats,
}

/// Session statistics
#[derive(Default)]
pub struct PlayerStats {
    pub messages_sent: usize,
    pub tools_run: usize,
    pub artifacts_created: usize,
    pub artifacts_collected: usize,
}

impl PlayerSession {
    pub fn new(username: String) -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            username,
            connected_at: Utc::now(),
            current_room: None,
            inventory: Vec::new(),
            stats: PlayerStats::default(),
        }
    }

    /// Generate the prompt string based on current state
    pub fn prompt(&self) -> String {
        match &self.current_room {
            Some(room) => format!("{}>", room),
            None => "lobby>".to_string(),
        }
    }

    /// Join a room
    pub fn join_room(&mut self, room: String) {
        self.current_room = Some(room);
    }

    /// Leave current room, return to lobby
    pub fn leave_room(&mut self) {
        self.current_room = None;
    }

    /// Add artifact to inventory
    pub fn pickup(&mut self, artifact_id: String) {
        if !self.inventory.contains(&artifact_id) {
            self.inventory.push(artifact_id);
            self.stats.artifacts_collected += 1;
        }
    }

    /// Remove artifact from inventory
    pub fn drop_artifact(&mut self, artifact_id: &str) -> bool {
        if let Some(pos) = self.inventory.iter().position(|a| a == artifact_id) {
            self.inventory.remove(pos);
            true
        } else {
            false
        }
    }

    /// Session duration
    pub fn session_duration(&self) -> chrono::Duration {
        Utc::now() - self.connected_at
    }
}
