//! World state: partylines (rooms) and their contents

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use crate::model::ModelHandle;

/// A partyline (room) where users and models interact
pub struct Partyline {
    pub id: PartylineId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub users: Vec<String>,
    pub models: Vec<ModelHandle>,
    pub artifacts: Vec<ArtifactRef>,
    pub history: Vec<Message>,
}

/// Unique identifier for a partyline
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PartylineId(pub Uuid);

impl PartylineId {
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

/// A message in the room history
#[derive(Debug, Clone)]
pub struct Message {
    pub id: usize,
    pub timestamp: DateTime<Utc>,
    pub sender: Sender,
    pub content: MessageContent,
}

#[derive(Debug, Clone)]
pub enum Sender {
    User(String),
    Model(String),
    System,
}

#[derive(Debug, Clone)]
pub enum MessageContent {
    Chat(String),
    Tell { to: String, message: String },
    ToolRun { tool: String, result: String },
    ArtifactCreated { artifact: ArtifactRef },
    Join(String),
    Leave(String),
}

impl Partyline {
    pub fn new(name: String) -> Self {
        Self {
            id: PartylineId::new(),
            name,
            description: None,
            created_at: Utc::now(),
            users: Vec::new(),
            models: Vec::new(),
            artifacts: Vec::new(),
            history: Vec::new(),
        }
    }

    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    pub fn add_user(&mut self, username: String) {
        if !self.users.contains(&username) {
            self.users.push(username.clone());
            self.add_message(Sender::System, MessageContent::Join(username));
        }
    }

    pub fn remove_user(&mut self, username: &str) {
        self.users.retain(|u| u != username);
        self.add_message(Sender::System, MessageContent::Leave(username.to_string()));
    }

    pub fn add_message(&mut self, sender: Sender, content: MessageContent) {
        let id = self.history.len();
        self.history.push(Message {
            id,
            timestamp: Utc::now(),
            sender,
            content,
        });
    }

    pub fn recent_history(&self, count: usize) -> &[Message] {
        let start = self.history.len().saturating_sub(count);
        &self.history[start..]
    }
}

/// Summary info for room listing
pub struct RoomSummary {
    pub name: String,
    pub user_count: usize,
    pub model_count: usize,
    pub artifact_count: usize,
}

/// The world: collection of partylines
pub struct World {
    pub rooms: HashMap<String, Partyline>,
}

impl World {
    pub fn new() -> Self {
        Self {
            rooms: HashMap::new(),
        }
    }

    pub fn create_room(&mut self, name: String) -> &Partyline {
        let room = Partyline::new(name.clone());
        self.rooms.insert(name.clone(), room);
        self.rooms.get(&name).unwrap()
    }

    pub fn get_room(&self, name: &str) -> Option<&Partyline> {
        self.rooms.get(name)
    }

    pub fn get_room_mut(&mut self, name: &str) -> Option<&mut Partyline> {
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
