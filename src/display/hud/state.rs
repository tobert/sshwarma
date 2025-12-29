//! HUD state types
//!
//! Data structures for tracking HUD state: participants, MCP connections,
//! room info, and notifications.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// Status of a participant (user or model) in a room
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParticipantStatus {
    /// Participant is idle/present
    Idle,
    /// Participant is thinking/processing
    Thinking,
    /// Participant is running a tool
    RunningTool(String),
    /// Participant has an error state
    Error(String),
    /// Participant is offline/away
    Offline,
    /// Custom emoji status
    Emoji(String),
}

impl Default for ParticipantStatus {
    fn default() -> Self {
        Self::Idle
    }
}

impl ParticipantStatus {
    /// Get the status glyph for HUD display
    pub fn glyph(&self) -> &str {
        match self {
            ParticipantStatus::Idle => "◇",
            ParticipantStatus::Thinking | ParticipantStatus::RunningTool(_) => "◈",
            ParticipantStatus::Error(_) => "◉",
            ParticipantStatus::Offline => "◌",
            ParticipantStatus::Emoji(e) => e.as_str(),
        }
    }

    /// Returns true if participant is actively working
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            ParticipantStatus::Thinking | ParticipantStatus::RunningTool(_)
        )
    }

    /// Short status text for display
    pub fn text(&self) -> String {
        match self {
            ParticipantStatus::Idle => String::new(), // Don't show "idle" for everyone
            ParticipantStatus::Thinking => "thinking".to_string(),
            ParticipantStatus::RunningTool(name) => format!("running {}", name),
            ParticipantStatus::Error(msg) => msg.chars().take(12).collect(),
            ParticipantStatus::Offline => "away".to_string(),
            ParticipantStatus::Emoji(_) => String::new(), // Emoji is self-explanatory
        }
    }
}

/// The kind of participant
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticipantKind {
    User,
    Model,
}

/// Presence of a participant in a room
#[derive(Debug, Clone)]
pub struct Presence {
    /// Participant name
    pub name: String,
    /// User or Model
    pub kind: ParticipantKind,
    /// Current status
    pub status: ParticipantStatus,
    /// When status was last updated
    pub updated_at: DateTime<Utc>,
}

impl Presence {
    pub fn user(name: String) -> Self {
        Self {
            name,
            kind: ParticipantKind::User,
            status: ParticipantStatus::Idle,
            updated_at: Utc::now(),
        }
    }

    pub fn model(name: String) -> Self {
        Self {
            name,
            kind: ParticipantKind::Model,
            status: ParticipantStatus::Idle,
            updated_at: Utc::now(),
        }
    }

    pub fn set_status(&mut self, status: ParticipantStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    pub fn is_model(&self) -> bool {
        self.kind == ParticipantKind::Model
    }

    pub fn is_user(&self) -> bool {
        self.kind == ParticipantKind::User
    }
}

/// A notification for the HUD bottom border
#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub ttl: Duration,
}

impl Notification {
    pub fn new(message: String, ttl_secs: i64) -> Self {
        Self {
            message,
            created_at: Utc::now(),
            ttl: Duration::seconds(ttl_secs),
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() - self.created_at >= self.ttl
    }
}

/// State of an MCP connection
#[derive(Debug, Clone)]
pub struct McpConnectionState {
    pub name: String,
    pub tool_count: usize,
    pub connected: bool,
    pub call_count: u64,
    pub last_tool: Option<String>,
}

/// Exit direction for room info display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExitDirection {
    North,
    East,
    South,
    West,
    Up,
    Down,
    Northeast,
    Southeast,
    Northwest,
    Southwest,
}

impl ExitDirection {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "n" | "north" => Some(Self::North),
            "e" | "east" => Some(Self::East),
            "s" | "south" => Some(Self::South),
            "w" | "west" => Some(Self::West),
            "u" | "up" => Some(Self::Up),
            "d" | "down" => Some(Self::Down),
            "ne" | "northeast" => Some(Self::Northeast),
            "se" | "southeast" => Some(Self::Southeast),
            "nw" | "northwest" => Some(Self::Northwest),
            "sw" | "southwest" => Some(Self::Southwest),
            _ => None,
        }
    }

    pub fn arrow(&self) -> char {
        match self {
            Self::North | Self::Up => '↑',
            Self::East => '→',
            Self::South | Self::Down => '↓',
            Self::West => '←',
            Self::Northeast => '↗',
            Self::Southeast => '↘',
            Self::Northwest => '↖',
            Self::Southwest => '↙',
        }
    }
}

/// Aggregated HUD state for a session
#[derive(Debug, Clone)]
pub struct HudState {
    /// Participants in current room (users and models mixed)
    pub participants: Vec<Presence>,
    /// Connected MCP servers with tool counts
    pub mcp_connections: Vec<McpConnectionState>,
    /// Current room name (None = lobby)
    pub room_name: Option<String>,
    /// Room description
    pub description: Option<String>,
    /// Room vibe (creative direction/mood)
    pub vibe: Option<String>,
    /// Exits from current room (direction string -> room name)
    pub exits: HashMap<String, String>,
    /// Session start time
    pub session_start: DateTime<Utc>,
    /// Current notification (if any)
    pub notification: Option<Notification>,
    /// Spinner frame (0-9 for braille spinner)
    pub spinner_frame: u8,
}

impl Default for HudState {
    fn default() -> Self {
        Self::new()
    }
}

impl HudState {
    pub fn new() -> Self {
        Self {
            participants: Vec::new(),
            mcp_connections: Vec::new(),
            room_name: None,
            description: None,
            vibe: None,
            exits: HashMap::new(),
            session_start: Utc::now(),
            notification: None,
            spinner_frame: 0,
        }
    }

    /// Add a user participant
    pub fn add_user(&mut self, name: String) {
        if !self.participants.iter().any(|p| p.name == name) {
            self.participants.push(Presence::user(name));
        }
    }

    /// Add a model participant
    pub fn add_model(&mut self, name: String) {
        if !self.participants.iter().any(|p| p.name == name) {
            self.participants.push(Presence::model(name));
        }
    }

    /// Remove a participant by name
    pub fn remove_participant(&mut self, name: &str) {
        self.participants.retain(|p| p.name != name);
    }

    /// Get a participant by name
    pub fn get_participant(&self, name: &str) -> Option<&Presence> {
        self.participants.iter().find(|p| p.name == name)
    }

    /// Get a mutable participant by name
    pub fn get_participant_mut(&mut self, name: &str) -> Option<&mut Presence> {
        self.participants.iter_mut().find(|p| p.name == name)
    }

    /// Update participant status
    pub fn update_status(&mut self, name: &str, status: ParticipantStatus) {
        if let Some(p) = self.get_participant_mut(name) {
            p.set_status(status);
        }
    }

    /// Push a notification (replaces existing)
    pub fn notify(&mut self, message: String, ttl_secs: i64) {
        self.notification = Some(Notification::new(message, ttl_secs));
    }

    /// Clear expired notification
    pub fn clear_expired_notification(&mut self) {
        if let Some(ref notif) = self.notification {
            if notif.is_expired() {
                self.notification = None;
            }
        }
    }

    /// Advance spinner frame
    pub fn advance_spinner(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % 10;
    }

    /// Get session duration
    pub fn session_duration(&self) -> Duration {
        Utc::now() - self.session_start
    }

    /// Format session duration as H:MM:SS
    pub fn duration_string(&self) -> String {
        let dur = self.session_duration();
        let secs = dur.num_seconds();
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let secs = secs % 60;
        format!("{}:{:02}:{:02}", hours, mins, secs)
    }

    /// Get exit arrows as a compact string
    pub fn exit_arrows(&self) -> String {
        let mut arrows = String::new();
        for dir_str in self.exits.keys() {
            if let Some(dir) = ExitDirection::from_str(dir_str) {
                arrows.push(dir.arrow());
            }
        }
        arrows
    }

    /// Count of active participants (thinking or running tool)
    pub fn active_count(&self) -> usize {
        self.participants
            .iter()
            .filter(|p| p.status.is_active())
            .count()
    }

    /// Count of users (not models)
    pub fn user_count(&self) -> usize {
        self.participants.iter().filter(|p| p.is_user()).count()
    }

    /// Count of models
    pub fn model_count(&self) -> usize {
        self.participants.iter().filter(|p| p.is_model()).count()
    }

    /// Set room context
    pub fn set_room(
        &mut self,
        name: Option<String>,
        description: Option<String>,
        vibe: Option<String>,
        exits: HashMap<String, String>,
    ) {
        self.room_name = name;
        self.description = description;
        self.vibe = vibe;
        self.exits = exits;
    }

    /// Set participants from lists of users and models
    pub fn set_participants(&mut self, users: Vec<String>, models: Vec<String>) {
        self.participants.clear();
        for name in users {
            self.participants.push(Presence::user(name));
        }
        for name in models {
            self.participants.push(Presence::model(name));
        }
    }

    /// Update MCP connections
    pub fn set_mcp_connections(&mut self, connections: Vec<McpConnectionState>) {
        self.mcp_connections = connections;
    }
}
