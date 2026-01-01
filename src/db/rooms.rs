//! Room CRUD operations
//!
//! Rooms are shared spaces where agents collaborate. Metadata lives in room_kv.

use super::{new_id, now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A room in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub created_at: i64,
}

impl Room {
    /// Create a new room
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            created_at: now_ms(),
        }
    }
}

// Database operations
impl Database {
    /// Insert a new room
    pub fn insert_room(&self, room: &Room) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO rooms (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![room.id, room.name, room.created_at],
        )
        .context("failed to insert room")?;
        Ok(())
    }

    /// Get room by ID
    pub fn get_room(&self, id: &str) -> Result<Option<Room>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT id, name, created_at FROM rooms WHERE id = ?1")
            .context("failed to prepare room query")?;

        let room = stmt
            .query_row(params![id], |row| {
                Ok(Room {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .optional()
            .context("failed to query room")?;

        Ok(room)
    }

    /// Get room by name
    pub fn get_room_by_name(&self, name: &str) -> Result<Option<Room>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT id, name, created_at FROM rooms WHERE name = ?1")
            .context("failed to prepare room query")?;

        let room = stmt
            .query_row(params![name], |row| {
                Ok(Room {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .optional()
            .context("failed to query room by name")?;

        Ok(room)
    }

    /// List all rooms
    pub fn list_rooms(&self) -> Result<Vec<Room>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT id, name, created_at FROM rooms ORDER BY name")
            .context("failed to prepare rooms query")?;

        let rooms = stmt
            .query([])?
            .mapped(|row| {
                Ok(Room {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list rooms")?;

        Ok(rooms)
    }

    /// Delete a room (cascades to room_kv)
    pub fn delete_room(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM room_kv WHERE room_id = ?1", params![id])?;
        conn.execute("DELETE FROM rooms WHERE id = ?1", params![id])
            .context("failed to delete room")?;
        Ok(())
    }

    // --- Room KV operations ---

    /// Set a room key-value pair
    pub fn set_room_kv(&self, room_id: &str, key: &str, value: Option<&str>) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO room_kv (room_id, key, value, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT (room_id, key) DO UPDATE SET value = ?3, updated_at = ?4
            "#,
            params![room_id, key, value, now_ms()],
        )
        .context("failed to set room kv")?;
        Ok(())
    }

    /// Get a room key-value pair
    pub fn get_room_kv(&self, room_id: &str, key: &str) -> Result<Option<String>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT value FROM room_kv WHERE room_id = ?1 AND key = ?2")
            .context("failed to prepare room kv query")?;

        let value = stmt
            .query_row(params![room_id, key], |row| row.get(0))
            .optional()
            .context("failed to query room kv")?;

        Ok(value)
    }

    /// Get all room key-value pairs
    pub fn get_all_room_kv(&self, room_id: &str) -> Result<HashMap<String, String>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT key, value FROM room_kv WHERE room_id = ?1 AND value IS NOT NULL")
            .context("failed to prepare room kv query")?;

        let mut kv = HashMap::new();
        let mut rows = stmt.query(params![room_id])?;
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let value: String = row.get(1)?;
            kv.insert(key, value);
        }

        Ok(kv)
    }

    /// Delete a room key-value pair
    pub fn delete_room_kv(&self, room_id: &str, key: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM room_kv WHERE room_id = ?1 AND key = ?2",
            params![room_id, key],
        )
        .context("failed to delete room kv")?;
        Ok(())
    }

    /// Get room exits (keys starting with "exit.")
    pub fn get_room_exits(&self, room_id: &str) -> Result<HashMap<String, String>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT key, value FROM room_kv WHERE room_id = ?1 AND key LIKE 'exit.%' AND value IS NOT NULL",
            )
            .context("failed to prepare exits query")?;

        let mut exits = HashMap::new();
        let mut rows = stmt.query(params![room_id])?;
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let value: String = row.get(1)?;
            // Strip "exit." prefix
            if let Some(direction) = key.strip_prefix("exit.") {
                exits.insert(direction.to_string(), value);
            }
        }

        Ok(exits)
    }

    /// Set a room exit
    pub fn set_room_exit(&self, room_id: &str, direction: &str, target_room: &str) -> Result<()> {
        let key = format!("exit.{}", direction);
        self.set_room_kv(room_id, &key, Some(target_room))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_room_crud() -> Result<()> {
        let db = Database::in_memory()?;

        let room = Room::new("lobby");
        db.insert_room(&room)?;

        let fetched = db.get_room(&room.id)?.expect("room should exist");
        assert_eq!(fetched.name, "lobby");

        let by_name = db.get_room_by_name("lobby")?.expect("should find by name");
        assert_eq!(by_name.id, room.id);

        let all = db.list_rooms()?;
        assert_eq!(all.len(), 1);

        db.delete_room(&room.id)?;
        assert!(db.get_room(&room.id)?.is_none());

        Ok(())
    }

    #[test]
    fn test_room_kv() -> Result<()> {
        let db = Database::in_memory()?;

        let room = Room::new("workshop");
        db.insert_room(&room)?;

        // Set and get
        db.set_room_kv(&room.id, "vibe", Some("collaborative coding"))?;
        let vibe = db.get_room_kv(&room.id, "vibe")?;
        assert_eq!(vibe, Some("collaborative coding".to_string()));

        // Update
        db.set_room_kv(&room.id, "vibe", Some("focused debugging"))?;
        let vibe = db.get_room_kv(&room.id, "vibe")?;
        assert_eq!(vibe, Some("focused debugging".to_string()));

        // Multiple keys
        db.set_room_kv(&room.id, "description", Some("A place for deep work"))?;
        let all = db.get_all_room_kv(&room.id)?;
        assert_eq!(all.len(), 2);
        assert_eq!(all.get("vibe"), Some(&"focused debugging".to_string()));

        // Delete
        db.delete_room_kv(&room.id, "vibe")?;
        assert!(db.get_room_kv(&room.id, "vibe")?.is_none());

        Ok(())
    }

    #[test]
    fn test_room_exits() -> Result<()> {
        let db = Database::in_memory()?;

        let lobby = Room::new("lobby");
        let studio = Room::new("studio");
        let garden = Room::new("garden");
        db.insert_room(&lobby)?;
        db.insert_room(&studio)?;
        db.insert_room(&garden)?;

        db.set_room_exit(&lobby.id, "north", "studio")?;
        db.set_room_exit(&lobby.id, "east", "garden")?;

        let exits = db.get_room_exits(&lobby.id)?;
        assert_eq!(exits.len(), 2);
        assert_eq!(exits.get("north"), Some(&"studio".to_string()));
        assert_eq!(exits.get("east"), Some(&"garden".to_string()));

        Ok(())
    }

    #[test]
    fn test_room_kv_cascades_on_delete() -> Result<()> {
        let db = Database::in_memory()?;

        let room = Room::new("temp");
        db.insert_room(&room)?;
        db.set_room_kv(&room.id, "key1", Some("value1"))?;
        db.set_room_kv(&room.id, "key2", Some("value2"))?;

        db.delete_room(&room.id)?;

        // KV should be gone too
        let all = db.get_all_room_kv(&room.id)?;
        assert!(all.is_empty());

        Ok(())
    }
}
