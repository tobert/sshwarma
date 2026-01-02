//! Participant status tracking
//!
//! Simple status tracker for models and users. Lua queries this via tools.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::RwLock;

/// Status of a participant
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Idle,
    Thinking,
    RunningTool(String),
    Error(String),
}

impl Default for Status {
    fn default() -> Self {
        Self::Idle
    }
}

impl Status {
    pub fn is_active(&self) -> bool {
        matches!(self, Status::Thinking | Status::RunningTool(_))
    }

    pub fn text(&self) -> String {
        match self {
            Status::Idle => String::new(),
            Status::Thinking => "thinking".to_string(),
            Status::RunningTool(name) => format!("running {}", name),
            Status::Error(msg) => msg.chars().take(20).collect(),
        }
    }

    pub fn glyph(&self) -> &'static str {
        match self {
            Status::Idle => "◇",
            Status::Thinking => "◈",
            Status::RunningTool(_) => "⚙",
            Status::Error(_) => "◉",
        }
    }
}

/// Entry tracking a participant's status
#[derive(Debug, Clone)]
struct StatusEntry {
    status: Status,
}

/// Thread-safe status tracker
#[derive(Debug, Default)]
pub struct StatusTracker {
    statuses: RwLock<HashMap<String, StatusEntry>>,
    session_start: DateTime<Utc>,
}

impl StatusTracker {
    pub fn new() -> Self {
        Self {
            statuses: RwLock::new(HashMap::new()),
            session_start: Utc::now(),
        }
    }

    /// Update a participant's status
    pub fn set(&self, name: &str, status: Status) {
        if let Ok(mut guard) = self.statuses.write() {
            guard.insert(name.to_string(), StatusEntry { status });
        }
    }

    /// Get a participant's status
    pub fn get(&self, name: &str) -> Status {
        self.statuses
            .read()
            .ok()
            .and_then(|guard| guard.get(name).map(|e| e.status.clone()))
            .unwrap_or_default()
    }

    /// Get all statuses as a snapshot
    pub fn snapshot(&self) -> HashMap<String, Status> {
        self.statuses
            .read()
            .map(|guard| guard.iter().map(|(k, v)| (k.clone(), v.status.clone())).collect())
            .unwrap_or_default()
    }

    /// Get session duration string
    pub fn duration_string(&self) -> String {
        let dur = Utc::now() - self.session_start;
        let secs = dur.num_seconds();
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let secs = secs % 60;
        format!("{}:{:02}:{:02}", hours, mins, secs)
    }

    /// Get session duration in milliseconds
    pub fn duration_ms(&self) -> i64 {
        (Utc::now() - self.session_start).num_milliseconds()
    }

    /// Get session start time
    pub fn session_start(&self) -> DateTime<Utc> {
        self.session_start
    }
}
