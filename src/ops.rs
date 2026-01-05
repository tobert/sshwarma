//! Core operations shared between slash commands and internal tools
//!
//! Pure async functions that take state + args and return Result<T>.
//! No formatting - callers decide how to present results.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde::Serialize;

use crate::state::SharedState;

/// Room summary for /look
#[derive(Debug, Clone, Serialize)]
pub struct RoomSummary {
    pub name: String,
    pub description: Option<String>,
    pub users: Vec<String>,
    pub models: Vec<String>,
    pub artifact_count: usize,
    pub vibe: Option<String>,
    pub exits: HashMap<String, String>,
}

/// Room list entry for /rooms
#[derive(Debug, Clone, Serialize)]
pub struct RoomInfo {
    pub name: String,
    pub user_count: usize,
}

/// Get room summary
pub async fn look(state: &SharedState, room_name: &str) -> Result<RoomSummary> {
    let world = state.world.read().await;
    let room = world
        .get_room(room_name)
        .ok_or_else(|| anyhow!("Room '{}' not found", room_name))?;

    let vibe = state.db.get_vibe(room_name).ok().flatten();
    let exits = state.db.get_exits(room_name).unwrap_or_default();

    Ok(RoomSummary {
        name: room.name.clone(),
        description: room.description.clone(),
        users: room.users.clone(),
        models: room.models.iter().map(|m| m.short_name.clone()).collect(),
        artifact_count: room.artifacts.len(),
        vibe,
        exits,
    })
}

/// Get users in room
pub async fn who(state: &SharedState, room_name: &str) -> Result<Vec<String>> {
    let world = state.world.read().await;
    let room = world
        .get_room(room_name)
        .ok_or_else(|| anyhow!("Room '{}' not found", room_name))?;

    Ok(room.users.clone())
}

/// List all rooms
pub async fn rooms(state: &SharedState) -> Result<Vec<RoomInfo>> {
    let world = state.world.read().await;
    let room_list = world.list_rooms();

    Ok(room_list
        .into_iter()
        .map(|r| RoomInfo {
            name: r.name,
            user_count: r.user_count,
        })
        .collect())
}

/// Get room history
pub async fn history(
    state: &SharedState,
    room_name: &str,
    limit: usize,
) -> Result<Vec<HistoryEntry>> {
    let messages = state.db.recent_messages(room_name, limit)?;

    Ok(messages
        .into_iter()
        .map(|m| HistoryEntry {
            timestamp: m.timestamp[11..16].to_string(), // HH:MM
            sender: m.sender_name,
            content: m.content,
        })
        .collect())
}

/// History entry
#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub timestamp: String,
    pub sender: String,
    pub content: String,
}

/// Get room exits
pub async fn exits(state: &SharedState, room_name: &str) -> Result<HashMap<String, String>> {
    state.db.get_exits(room_name)
}

/// List available MCP tools
pub async fn tools(state: &SharedState) -> Result<Vec<ToolInfo>> {
    let tool_list = state.mcp.list_tools().await;

    Ok(tool_list
        .into_iter()
        .map(|t| ToolInfo {
            name: t.name,
            source: t.source,
            description: t.description,
        })
        .collect())
}

/// Tool info
#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub source: String,
    pub description: String,
}

/// Get vibe for room
pub async fn get_vibe(state: &SharedState, room_name: &str) -> Result<Option<String>> {
    state.db.get_vibe(room_name)
}

/// Set vibe for room
pub async fn set_vibe(state: &SharedState, room_name: &str, vibe: &str) -> Result<()> {
    state.db.set_vibe(room_name, Some(vibe))?;

    // Update in-memory state
    let mut world = state.world.write().await;
    if let Some(room) = world.get_room_mut(room_name) {
        room.context.vibe = Some(vibe.to_string());
    }

    Ok(())
}

/// Get navigation enabled for room (defaults to true)
pub async fn get_room_navigation(state: &SharedState, room_name: &str) -> Result<bool> {
    state.db.get_room_navigation(room_name)
}

/// Set navigation enabled for room
pub async fn set_room_navigation(
    state: &SharedState,
    room_name: &str,
    enabled: bool,
) -> Result<()> {
    state.db.set_room_navigation(room_name, enabled)?;
    Ok(())
}

/// Say something to the room
pub async fn say(state: &SharedState, room_name: &str, sender: &str, message: &str) -> Result<()> {
    use crate::db::rows::Row;

    // Get or create the room's buffer
    let buffer = state.db.get_or_create_room_buffer(room_name)?;

    // Get agent ID for sender (create if needed)
    let agent_id = state.db.get_or_create_human_agent(sender)?.id;

    // Create and add the row
    let mut row = Row::message(&buffer.id, &agent_id, message, false);
    state.db.append_row(&mut row)?;

    Ok(())
}

/// Journal entry kinds
#[derive(Debug, Clone, Copy, Serialize)]
pub enum JournalKind {
    Note,
    Decision,
    Idea,
    Milestone,
}

impl JournalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            JournalKind::Note => "note",
            JournalKind::Decision => "decision",
            JournalKind::Idea => "idea",
            JournalKind::Milestone => "milestone",
        }
    }
}

/// Add journal entry
pub async fn add_journal(
    state: &SharedState,
    room_name: &str,
    author: &str,
    content: &str,
    kind: JournalKind,
) -> Result<()> {
    let db_kind = match kind {
        JournalKind::Note => crate::world::JournalKind::Note,
        JournalKind::Decision => crate::world::JournalKind::Decision,
        JournalKind::Idea => crate::world::JournalKind::Idea,
        JournalKind::Milestone => crate::world::JournalKind::Milestone,
    };

    state
        .db
        .add_journal_entry(room_name, author, content, db_kind)?;
    Ok(())
}

/// Journal entry from database
#[derive(Debug, Clone, Serialize)]
pub struct JournalEntry {
    pub timestamp: String,
    pub kind: String,
    pub author: String,
    pub content: String,
}

/// Get journal entries
pub async fn get_journal(
    state: &SharedState,
    room_name: &str,
    kind_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<JournalEntry>> {
    let db_kind = kind_filter.and_then(crate::world::JournalKind::parse);
    let entries = state.db.get_journal_entries(room_name, db_kind, limit)?;

    Ok(entries
        .into_iter()
        .map(|e| JournalEntry {
            timestamp: e.timestamp.format("%m-%d %H:%M").to_string(),
            kind: e.kind.to_string(),
            author: e.author,
            content: e.content,
        })
        .collect())
}

/// Add inspiration
pub async fn add_inspiration(
    state: &SharedState,
    room_name: &str,
    content: &str,
    added_by: &str,
) -> Result<()> {
    state.db.add_inspiration(room_name, content, added_by)?;
    Ok(())
}

/// Inspiration entry
#[derive(Debug, Clone, Serialize)]
pub struct Inspiration {
    pub content: String,
}

/// Get inspirations
pub async fn get_inspirations(state: &SharedState, room_name: &str) -> Result<Vec<Inspiration>> {
    let inspirations = state.db.get_inspirations(room_name)?;
    Ok(inspirations
        .into_iter()
        .map(|i| Inspiration { content: i.content })
        .collect())
}

/// Asset binding info
#[derive(Debug, Clone, Serialize)]
pub struct AssetBinding {
    pub role: String,
    pub artifact_id: String,
    pub notes: Option<String>,
    pub bound_by: String,
    pub bound_at: String,
}

/// Get asset binding
pub async fn examine_asset(
    state: &SharedState,
    room_name: &str,
    role: &str,
) -> Result<Option<AssetBinding>> {
    match state.db.get_asset_binding(room_name, role)? {
        Some(b) => Ok(Some(AssetBinding {
            role: b.role,
            artifact_id: b.artifact_id,
            notes: b.notes,
            bound_by: b.bound_by,
            bound_at: b.bound_at.format("%Y-%m-%d %H:%M").to_string(),
        })),
        None => Ok(None),
    }
}

/// Bind asset to role
pub async fn bind_asset(
    state: &SharedState,
    room_name: &str,
    role: &str,
    artifact_id: &str,
    bound_by: &str,
) -> Result<()> {
    state
        .db
        .bind_asset(room_name, role, artifact_id, None, bound_by)?;
    Ok(())
}

/// Unbind asset from role
pub async fn unbind_asset(state: &SharedState, room_name: &str, role: &str) -> Result<()> {
    state.db.unbind_asset(room_name, role)?;
    Ok(())
}

// Navigation operations

/// Join a room
pub async fn join(
    state: &SharedState,
    username: &str,
    current_room: Option<&str>,
    target_room: &str,
) -> Result<RoomSummary> {
    tracing::info!("ops::join: entering, target={}", target_room);

    // Leave current room if in one
    if let Some(current) = current_room {
        tracing::info!("ops::join: leaving current room {}", current);
        let mut world = state.world.write().await;
        tracing::info!("ops::join: got write lock for leave");
        if let Some(room) = world.get_room_mut(current) {
            room.remove_user(username);
        }
    }

    // Check target exists
    tracing::info!("ops::join: checking target exists");
    {
        tracing::info!("ops::join: acquiring read lock");
        let world = state.world.read().await;
        tracing::info!("ops::join: got read lock");
        if world.get_room(target_room).is_none() {
            return Err(anyhow!(
                "No room named '{}'. Use /create {} to make one.",
                target_room,
                target_room
            ));
        }
    }
    tracing::info!("ops::join: target exists");

    // Ensure room buffer exists in database
    tracing::info!("ops::join: getting room buffer");
    let buffer = state.db.get_or_create_room_buffer(target_room)?;
    tracing::info!("ops::join: got room buffer");

    // Join target room
    tracing::info!("ops::join: acquiring write lock for join");
    {
        let mut world = state.world.write().await;
        tracing::info!("ops::join: got write lock for join");
        if let Some(room) = world.get_room_mut(target_room) {
            room.add_user(username.to_string());
            // Set buffer ID if not already set
            if room.buffer_id.is_none() {
                room.set_buffer_id(buffer.id.clone());
            }
        }
    }

    // Return room info
    look(state, target_room).await
}

/// Leave room (return to lobby)
pub async fn leave(state: &SharedState, username: &str, room_name: &str) -> Result<()> {
    let mut world = state.world.write().await;
    if let Some(room) = world.get_room_mut(room_name) {
        room.remove_user(username);
    }
    Ok(())
}

/// Create a new room
pub async fn create_room(
    state: &SharedState,
    username: &str,
    room_name: &str,
    current_room: Option<&str>,
) -> Result<RoomSummary> {
    // Validate name
    if !room_name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow!(
            "Room name can only contain letters, numbers, dashes, and underscores."
        ));
    }

    // Check if exists
    {
        let world = state.world.read().await;
        if world.get_room(room_name).is_some() {
            return Err(anyhow!(
                "Room '{}' already exists. Use /join {} to enter.",
                room_name,
                room_name
            ));
        }
    }

    // Leave current room
    if let Some(current) = current_room {
        let mut world = state.world.write().await;
        if let Some(room) = world.get_room_mut(current) {
            room.remove_user(username);
        }
    }

    // Create room
    {
        let mut world = state.world.write().await;
        world.create_room(room_name.to_string());
        if let Some(room) = world.get_room_mut(room_name) {
            room.add_user(username.to_string());
        }
    }

    state.db.create_room(room_name, None)?;

    look(state, room_name).await
}

/// Fork a room (copy context)
pub async fn fork_room(
    state: &SharedState,
    username: &str,
    source_room: &str,
    new_room: &str,
) -> Result<RoomSummary> {
    // Validate name
    if !new_room
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow!(
            "Room name can only contain letters, numbers, dashes, and underscores."
        ));
    }

    // Check target doesn't exist
    {
        let world = state.world.read().await;
        if world.get_room(new_room).is_some() {
            return Err(anyhow!("Room '{}' already exists.", new_room));
        }
    }

    // Fork in database
    state.db.fork_room(source_room, new_room)?;

    // Create in memory and join
    {
        let mut world = state.world.write().await;
        world.create_room(new_room.to_string());

        // Leave source room
        if let Some(room) = world.get_room_mut(source_room) {
            room.remove_user(username);
        }

        // Join new room
        if let Some(room) = world.get_room_mut(new_room) {
            room.add_user(username.to_string());
        }
    }

    look(state, new_room).await
}

/// Navigate via exit
pub async fn go(
    state: &SharedState,
    username: &str,
    current_room: &str,
    direction: &str,
) -> Result<RoomSummary> {
    let exits = state.db.get_exits(current_room)?;

    match exits.get(direction) {
        Some(target) => join(state, username, Some(current_room), target).await,
        None => {
            if exits.is_empty() {
                Err(anyhow!("No exits from this room."))
            } else {
                let available: Vec<_> = exits.keys().collect();
                Err(anyhow!(
                    "No exit '{}'. Available: {}",
                    direction,
                    available
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
    }
}

/// Dig an exit
pub async fn dig(
    state: &SharedState,
    from_room: &str,
    direction: &str,
    to_room: &str,
) -> Result<String> {
    // Create exit
    state.db.add_exit(from_room, direction, to_room)?;

    // Create reverse exit
    let reverse = match direction {
        "north" => "south",
        "south" => "north",
        "east" => "west",
        "west" => "east",
        "up" => "down",
        "down" => "up",
        "in" => "out",
        "out" => "in",
        _ => "back",
    };

    state.db.add_exit(to_room, reverse, from_room)?;

    Ok(reverse.to_string())
}
