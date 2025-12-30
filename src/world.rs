//! World state: rooms and their contents

use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::display::{EntryContent, EntrySource, Ledger};
use crate::model::ModelHandle;

/// A room where users and models interact
pub struct Room {
    pub id: RoomId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub users: Vec<String>,
    pub models: Vec<ModelHandle>,
    pub artifacts: Vec<ArtifactRef>,
    /// Room's conversation ledger - the authoritative history
    pub ledger: Ledger,
    pub context: RoomContext,
}

/// Unique identifier for a room
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoomId(pub Uuid);

impl RoomId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Reference to an artifact in the room
#[derive(Debug, Clone)]
pub struct ArtifactRef {
    pub id: String,
    pub artifact_type: ArtifactType,
    pub name: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum ArtifactType {
    Midi,
    Wav,
    Text,
    Image,
    Other(String),
}

/// Rich context for a room beyond basic metadata
#[derive(Debug, Clone, Default)]
pub struct RoomContext {
    /// The vibe - atmosphere, mood, creative direction
    pub vibe: Option<String>,
    /// Journal entries - intentional notes, not chat noise
    pub journal: Vec<JournalEntry>,
    /// Named asset bindings - semantic roles like "drums", "main_theme"
    pub assets: HashMap<String, AssetBinding>,
    /// Inspiration board - references, links, ideas, moods
    pub inspirations: Vec<Inspiration>,
    /// Exits to other rooms (direction -> room_name)
    pub exits: HashMap<String, String>,
    /// Freeform tags for categorization
    pub tags: HashSet<String>,
    /// Parent room for fork DAG
    pub parent: Option<String>,
    /// Profiles that customize model behavior
    pub profiles: Vec<Profile>,
    /// Role assignments: model_handle -> [role_names]
    pub role_assignments: HashMap<String, Vec<String>>,
}

/// A journal entry - intentional documentation of the creative process
#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub author: String,
    pub content: String,
    pub kind: JournalKind,
}

/// Types of journal entries
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalKind {
    /// General observation
    Note,
    /// "We decided to..."
    Decision,
    /// "We finished..."
    Milestone,
    /// "What if..."
    Idea,
    /// Open thread to explore
    Question,
}

impl JournalKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "note" => Some(Self::Note),
            "decision" | "decide" => Some(Self::Decision),
            "milestone" => Some(Self::Milestone),
            "idea" => Some(Self::Idea),
            "question" => Some(Self::Question),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Decision => "decision",
            Self::Milestone => "milestone",
            Self::Idea => "idea",
            Self::Question => "question",
        }
    }
}

impl std::fmt::Display for JournalKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// An asset bound to a room with a semantic role
#[derive(Debug, Clone)]
pub struct AssetBinding {
    pub artifact_id: String,
    pub role: String,
    pub notes: Option<String>,
    pub bound_by: String,
    pub bound_at: DateTime<Utc>,
}

/// An inspiration on the room's mood board
#[derive(Debug, Clone)]
pub struct Inspiration {
    pub id: String,
    pub content: String,
    pub added_by: String,
    pub added_at: DateTime<Utc>,
}

/// A profile that customizes model behavior in a room
#[derive(Debug, Clone)]
pub struct Profile {
    /// Unique identifier within the room
    pub name: String,
    /// What this profile targets
    pub target: ProfileTarget,
    /// System prompt addition (goes to preamble)
    pub system_prompt: Option<String>,
    /// Context prefix (prepended to dynamic context)
    pub context_prefix: Option<String>,
    /// Context suffix (appended to dynamic context)
    pub context_suffix: Option<String>,
    /// Priority for stacking (lower = applied first)
    pub priority: i32,
}

/// What a profile applies to
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileTarget {
    /// Applies to specific model by handle (e.g., "qwen-4b")
    Model(String),
    /// Applies to any model with this role assigned (e.g., "creative")
    Role(String),
    /// Applies to all models in the room
    Room,
}

impl Room {
    pub fn new(name: String) -> Self {
        Self {
            id: RoomId::new(),
            name,
            description: None,
            created_at: Utc::now(),
            users: Vec::new(),
            models: Vec::new(),
            artifacts: Vec::new(),
            ledger: Ledger::new(500),
            context: RoomContext::default(),
        }
    }

    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    pub fn add_user(&mut self, username: String) {
        use crate::display::PresenceAction;
        if !self.users.contains(&username) {
            self.users.push(username.clone());
            self.ledger.push(
                EntrySource::System,
                EntryContent::Presence {
                    user: username,
                    action: PresenceAction::Join,
                },
            );
        }
    }

    pub fn remove_user(&mut self, username: &str) {
        use crate::display::PresenceAction;
        self.users.retain(|u| u != username);
        self.ledger.push(
            EntrySource::System,
            EntryContent::Presence {
                user: username.to_string(),
                action: PresenceAction::Leave,
            },
        );
    }

    /// Add an entry to the room's ledger
    pub fn add_entry(&mut self, source: EntrySource, content: EntryContent) -> crate::display::EntryId {
        self.ledger.push(source, content)
    }

    /// Load ledger entries from DB (call once when room is first accessed)
    pub fn load_entries_from_db(&mut self, entries: &[crate::display::LedgerEntry]) {
        for entry in entries {
            self.ledger.push(entry.source.clone(), entry.content.clone());
        }
    }

    /// Load history from legacy messages table into ledger
    /// Used for backward compatibility with existing databases
    pub fn load_history_from_db(&mut self, messages: &[crate::db::MessageRow]) {
        for msg in messages {
            let source = match msg.sender_type.as_str() {
                "model" => EntrySource::Model {
                    name: msg.sender_name.clone(),
                    is_streaming: false,
                },
                "system" => EntrySource::System,
                _ => EntrySource::User(msg.sender_name.clone()),
            };
            self.ledger.push(source, EntryContent::Chat(msg.content.clone()));
        }
    }
}

/// Summary info for room listing
pub struct RoomSummary {
    pub name: String,
    pub user_count: usize,
    pub model_count: usize,
    pub artifact_count: usize,
}

/// The world: collection of rooms
pub struct World {
    pub rooms: HashMap<String, Room>,
}

impl World {
    pub fn new() -> Self {
        Self {
            rooms: HashMap::new(),
        }
    }

    pub fn create_room(&mut self, name: String) -> &Room {
        let room = Room::new(name.clone());
        self.rooms.insert(name.clone(), room);
        self.rooms.get(&name).unwrap()
    }

    pub fn get_room(&self, name: &str) -> Option<&Room> {
        self.rooms.get(name)
    }

    pub fn get_room_mut(&mut self, name: &str) -> Option<&mut Room> {
        self.rooms.get_mut(name)
    }

    pub fn list_rooms(&self) -> Vec<RoomSummary> {
        self.rooms
            .values()
            .map(|r| RoomSummary {
                name: r.name.clone(),
                user_count: r.users.len(),
                model_count: r.models.len(),
                artifact_count: r.artifacts.len(),
            })
            .collect()
    }
}
