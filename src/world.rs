//! World state: rooms and their contents
//!
//! Rooms are in-memory representations synced with the database.
//! Messages are stored in the Row/Buffer system, not in Room structs.

use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

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
    /// Buffer ID for the room's main chat buffer (from database)
    /// None if the room buffer hasn't been created yet
    pub buffer_id: Option<String>,
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

impl Default for RoomId {
    fn default() -> Self {
        Self::new()
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
    /// Named asset bindings - semantic roles like "drums", "main_theme"
    pub assets: HashMap<String, AssetBinding>,
    /// Exits to other rooms (direction -> room_name)
    pub exits: HashMap<String, String>,
    /// Freeform tags for categorization
    pub tags: HashSet<String>,
    /// Parent room for fork DAG
    pub parent: Option<String>,
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
            buffer_id: None,
            context: RoomContext::default(),
        }
    }

    /// Create a room with an existing buffer ID (from database)
    pub fn with_buffer(name: String, buffer_id: String) -> Self {
        Self {
            id: RoomId::new(),
            name,
            description: None,
            created_at: Utc::now(),
            users: Vec::new(),
            models: Vec::new(),
            artifacts: Vec::new(),
            buffer_id: Some(buffer_id),
            context: RoomContext::default(),
        }
    }

    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    /// Add user to in-memory list
    /// Note: Caller should add presence.join row to the buffer
    pub fn add_user(&mut self, username: String) {
        if !self.users.contains(&username) {
            self.users.push(username);
        }
    }

    /// Remove user from in-memory list
    /// Note: Caller should add presence.leave row to the buffer
    pub fn remove_user(&mut self, username: &str) {
        self.users.retain(|u| u != username);
    }

    /// Get the buffer ID, panicking if not set
    /// Use this when you're certain the buffer has been initialized
    pub fn buffer_id(&self) -> &str {
        self.buffer_id
            .as_deref()
            .expect("room buffer not initialized")
    }

    /// Set the buffer ID (typically after creating via db.get_or_create_room_buffer)
    pub fn set_buffer_id(&mut self, buffer_id: String) {
        self.buffer_id = Some(buffer_id);
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

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
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

    /// Create a room with an existing buffer ID
    pub fn create_room_with_buffer(&mut self, name: String, buffer_id: String) -> &Room {
        let room = Room::with_buffer(name.clone(), buffer_id);
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
