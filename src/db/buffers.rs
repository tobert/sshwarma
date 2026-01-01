//! Buffer CRUD operations
//!
//! Buffers are containers for rows. Can be room chat, thinking, tool output, scratch.

use super::{new_id, now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Buffer type discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BufferType {
    RoomChat,
    Thinking,
    ToolOutput,
    Scratch,
}

impl BufferType {
    pub fn as_str(&self) -> &'static str {
        match self {
            BufferType::RoomChat => "room_chat",
            BufferType::Thinking => "thinking",
            BufferType::ToolOutput => "tool_output",
            BufferType::Scratch => "scratch",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "room_chat" => Some(BufferType::RoomChat),
            "thinking" => Some(BufferType::Thinking),
            "tool_output" => Some(BufferType::ToolOutput),
            "scratch" => Some(BufferType::Scratch),
            _ => None,
        }
    }
}

/// Tombstone status for collapsed buffers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TombstoneStatus {
    Success,
    Failure,
    Cancelled,
}

impl TombstoneStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TombstoneStatus::Success => "success",
            TombstoneStatus::Failure => "failure",
            TombstoneStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "success" => Some(TombstoneStatus::Success),
            "failure" => Some(TombstoneStatus::Failure),
            "cancelled" => Some(TombstoneStatus::Cancelled),
            _ => None,
        }
    }
}

/// A buffer in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Buffer {
    pub id: String,
    pub room_id: Option<String>,
    pub owner_agent_id: Option<String>,
    pub buffer_type: BufferType,
    pub created_at: i64,

    // Tombstoning
    pub tombstoned: bool,
    pub tombstone_status: Option<TombstoneStatus>,
    pub tombstone_summary: Option<String>,
    pub tombstoned_at: Option<i64>,

    // Forking
    pub parent_buffer_id: Option<String>,

    // Wrap behavior
    pub include_in_wrap: bool,
    pub wrap_priority: i32,
}

impl Buffer {
    /// Create a new buffer
    pub fn new(buffer_type: BufferType) -> Self {
        Self {
            id: new_id(),
            room_id: None,
            owner_agent_id: None,
            buffer_type,
            created_at: now_ms(),
            tombstoned: false,
            tombstone_status: None,
            tombstone_summary: None,
            tombstoned_at: None,
            parent_buffer_id: None,
            include_in_wrap: true,
            wrap_priority: 100,
        }
    }

    /// Create a room chat buffer
    pub fn room_chat(room_id: impl Into<String>) -> Self {
        let mut buf = Self::new(BufferType::RoomChat);
        buf.room_id = Some(room_id.into());
        buf
    }

    /// Create a thinking buffer for an agent
    pub fn thinking(room_id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        let mut buf = Self::new(BufferType::Thinking);
        buf.room_id = Some(room_id.into());
        buf.owner_agent_id = Some(agent_id.into());
        buf.include_in_wrap = false; // Thinking not included by default
        buf
    }

    /// Create a tool output buffer
    pub fn tool_output(room_id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        let mut buf = Self::new(BufferType::ToolOutput);
        buf.room_id = Some(room_id.into());
        buf.owner_agent_id = Some(agent_id.into());
        buf
    }

    /// Tombstone this buffer
    pub fn tombstone(&mut self, status: TombstoneStatus, summary: Option<String>) {
        self.tombstoned = true;
        self.tombstone_status = Some(status);
        self.tombstone_summary = summary;
        self.tombstoned_at = Some(now_ms());
    }
}

// Database operations
impl Database {
    /// Insert a new buffer
    pub fn insert_buffer(&self, buffer: &Buffer) -> Result<()> {
        let conn = self.conn()?;
        let tombstone_status = buffer.tombstone_status.as_ref().map(|s| s.as_str());

        conn.execute(
            r#"
            INSERT INTO buffers (
                id, room_id, owner_agent_id, buffer_type, created_at,
                tombstoned, tombstone_status, tombstone_summary, tombstoned_at,
                parent_buffer_id, include_in_wrap, wrap_priority
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                buffer.id,
                buffer.room_id,
                buffer.owner_agent_id,
                buffer.buffer_type.as_str(),
                buffer.created_at,
                buffer.tombstoned as i32,
                tombstone_status,
                buffer.tombstone_summary,
                buffer.tombstoned_at,
                buffer.parent_buffer_id,
                buffer.include_in_wrap as i32,
                buffer.wrap_priority,
            ],
        )
        .context("failed to insert buffer")?;
        Ok(())
    }

    /// Get buffer by ID
    pub fn get_buffer(&self, id: &str) -> Result<Option<Buffer>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, room_id, owner_agent_id, buffer_type, created_at,
                   tombstoned, tombstone_status, tombstone_summary, tombstoned_at,
                   parent_buffer_id, include_in_wrap, wrap_priority
            FROM buffers WHERE id = ?1
            "#,
            )
            .context("failed to prepare buffer query")?;

        let buffer = stmt
            .query_row(params![id], |row| Self::buffer_from_row(row))
            .optional()
            .context("failed to query buffer")?;

        Ok(buffer)
    }

    /// List buffers for a room
    pub fn list_room_buffers(&self, room_id: &str) -> Result<Vec<Buffer>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, room_id, owner_agent_id, buffer_type, created_at,
                   tombstoned, tombstone_status, tombstone_summary, tombstoned_at,
                   parent_buffer_id, include_in_wrap, wrap_priority
            FROM buffers WHERE room_id = ?1
            ORDER BY created_at
            "#,
            )
            .context("failed to prepare buffers query")?;

        let buffers = stmt
            .query(params![room_id])?
            .mapped(|row| Self::buffer_from_row(row))
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list buffers")?;

        Ok(buffers)
    }

    /// List buffers for a room, filtered by type
    pub fn list_room_buffers_by_type(
        &self,
        room_id: &str,
        buffer_type: BufferType,
    ) -> Result<Vec<Buffer>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, room_id, owner_agent_id, buffer_type, created_at,
                   tombstoned, tombstone_status, tombstone_summary, tombstoned_at,
                   parent_buffer_id, include_in_wrap, wrap_priority
            FROM buffers WHERE room_id = ?1 AND buffer_type = ?2
            ORDER BY created_at
            "#,
            )
            .context("failed to prepare buffers query")?;

        let buffers = stmt
            .query(params![room_id, buffer_type.as_str()])?
            .mapped(|row| Self::buffer_from_row(row))
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list buffers by type")?;

        Ok(buffers)
    }

    /// Get the main room chat buffer for a room (creates if needed)
    pub fn get_or_create_room_chat_buffer(&self, room_id: &str) -> Result<Buffer> {
        let buffers = self.list_room_buffers_by_type(room_id, BufferType::RoomChat)?;
        if let Some(buf) = buffers.into_iter().next() {
            return Ok(buf);
        }

        // Create a new one
        let buf = Buffer::room_chat(room_id);
        self.insert_buffer(&buf)?;
        Ok(buf)
    }

    /// Update buffer (for tombstoning, etc.)
    pub fn update_buffer(&self, buffer: &Buffer) -> Result<()> {
        let conn = self.conn()?;
        let tombstone_status = buffer.tombstone_status.as_ref().map(|s| s.as_str());

        conn.execute(
            r#"
            UPDATE buffers SET
                room_id = ?2, owner_agent_id = ?3, buffer_type = ?4,
                tombstoned = ?5, tombstone_status = ?6, tombstone_summary = ?7, tombstoned_at = ?8,
                parent_buffer_id = ?9, include_in_wrap = ?10, wrap_priority = ?11
            WHERE id = ?1
            "#,
            params![
                buffer.id,
                buffer.room_id,
                buffer.owner_agent_id,
                buffer.buffer_type.as_str(),
                buffer.tombstoned as i32,
                tombstone_status,
                buffer.tombstone_summary,
                buffer.tombstoned_at,
                buffer.parent_buffer_id,
                buffer.include_in_wrap as i32,
                buffer.wrap_priority,
            ],
        )
        .context("failed to update buffer")?;
        Ok(())
    }

    /// Delete a buffer (cascades to rows)
    pub fn delete_buffer(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM buffers WHERE id = ?1", params![id])
            .context("failed to delete buffer")?;
        Ok(())
    }

    fn buffer_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Buffer> {
        let type_str: String = row.get(3)?;
        let tombstoned_int: i32 = row.get(5)?;
        let tombstone_status_str: Option<String> = row.get(6)?;
        let include_in_wrap_int: i32 = row.get(10)?;

        Ok(Buffer {
            id: row.get(0)?,
            room_id: row.get(1)?,
            owner_agent_id: row.get(2)?,
            buffer_type: BufferType::from_str(&type_str).unwrap_or(BufferType::Scratch),
            created_at: row.get(4)?,
            tombstoned: tombstoned_int != 0,
            tombstone_status: tombstone_status_str.and_then(|s| TombstoneStatus::from_str(&s)),
            tombstone_summary: row.get(7)?,
            tombstoned_at: row.get(8)?,
            parent_buffer_id: row.get(9)?,
            include_in_wrap: include_in_wrap_int != 0,
            wrap_priority: row.get(11)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{agents::Agent, agents::AgentKind, rooms::Room};

    #[test]
    fn test_buffer_crud() -> Result<()> {
        let db = Database::in_memory()?;

        let room = Room::new("lobby");
        db.insert_room(&room)?;

        let buffer = Buffer::room_chat(&room.id);
        db.insert_buffer(&buffer)?;

        let fetched = db.get_buffer(&buffer.id)?.expect("buffer should exist");
        assert_eq!(fetched.buffer_type, BufferType::RoomChat);
        assert_eq!(fetched.room_id, Some(room.id.clone()));
        assert!(!fetched.tombstoned);
        assert!(fetched.include_in_wrap);

        let room_buffers = db.list_room_buffers(&room.id)?;
        assert_eq!(room_buffers.len(), 1);

        db.delete_buffer(&buffer.id)?;
        assert!(db.get_buffer(&buffer.id)?.is_none());

        Ok(())
    }

    #[test]
    fn test_buffer_types() -> Result<()> {
        let db = Database::in_memory()?;

        let room = Room::new("workshop");
        db.insert_room(&room)?;

        let agent = Agent::new("agent1", AgentKind::Model);
        db.insert_agent(&agent)?;

        let chat = Buffer::room_chat(&room.id);
        let thinking = Buffer::thinking(&room.id, &agent.id);
        let tool = Buffer::tool_output(&room.id, &agent.id);

        db.insert_buffer(&chat)?;
        db.insert_buffer(&thinking)?;
        db.insert_buffer(&tool)?;

        let all = db.list_room_buffers(&room.id)?;
        assert_eq!(all.len(), 3);

        let chats = db.list_room_buffers_by_type(&room.id, BufferType::RoomChat)?;
        assert_eq!(chats.len(), 1);

        let thinks = db.list_room_buffers_by_type(&room.id, BufferType::Thinking)?;
        assert_eq!(thinks.len(), 1);
        assert!(!thinks[0].include_in_wrap); // Thinking excluded by default

        Ok(())
    }

    #[test]
    fn test_buffer_tombstoning() -> Result<()> {
        let db = Database::in_memory()?;

        let room = Room::new("temp");
        db.insert_room(&room)?;

        let agent = Agent::new("agent1", AgentKind::Model);
        db.insert_agent(&agent)?;

        let mut buffer = Buffer::tool_output(&room.id, &agent.id);
        db.insert_buffer(&buffer)?;

        buffer.tombstone(TombstoneStatus::Success, Some("Completed successfully".to_string()));
        db.update_buffer(&buffer)?;

        let fetched = db.get_buffer(&buffer.id)?.expect("buffer should exist");
        assert!(fetched.tombstoned);
        assert_eq!(fetched.tombstone_status, Some(TombstoneStatus::Success));
        assert_eq!(
            fetched.tombstone_summary,
            Some("Completed successfully".to_string())
        );
        assert!(fetched.tombstoned_at.is_some());

        Ok(())
    }

    #[test]
    fn test_get_or_create_room_chat() -> Result<()> {
        let db = Database::in_memory()?;

        let room = Room::new("studio");
        db.insert_room(&room)?;

        // First call creates
        let buf1 = db.get_or_create_room_chat_buffer(&room.id)?;
        assert_eq!(buf1.buffer_type, BufferType::RoomChat);

        // Second call returns same
        let buf2 = db.get_or_create_room_chat_buffer(&room.id)?;
        assert_eq!(buf1.id, buf2.id);

        Ok(())
    }
}
