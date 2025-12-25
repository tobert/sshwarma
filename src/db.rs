//! Persistence: sqlite for sessions, history, room state

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

/// Database handle
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create database at path
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Open in-memory database (for testing)
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS rooms (
                name TEXT PRIMARY KEY,
                description TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                room TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                sender_type TEXT NOT NULL,
                sender_name TEXT NOT NULL,
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                FOREIGN KEY (room) REFERENCES rooms(name)
            );

            CREATE TABLE IF NOT EXISTS artifacts (
                id TEXT PRIMARY KEY,
                room TEXT NOT NULL,
                artifact_type TEXT NOT NULL,
                name TEXT NOT NULL,
                created_by TEXT NOT NULL,
                created_at TEXT NOT NULL,
                cas_hash TEXT,
                FOREIGN KEY (room) REFERENCES rooms(name)
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                username TEXT NOT NULL,
                connected_at TEXT NOT NULL,
                disconnected_at TEXT,
                current_room TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_messages_room ON messages(room);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
            CREATE INDEX IF NOT EXISTS idx_artifacts_room ON artifacts(room);
            "#,
        )?;

        Ok(())
    }

    /// Create a room
    pub fn create_room(&self, name: &str, description: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO rooms (name, description, created_at) VALUES (?1, ?2, datetime('now'))",
            params![name, description],
        )?;
        Ok(())
    }

    /// Get room names
    pub fn list_rooms(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT name FROM rooms ORDER BY name")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut names = Vec::new();
        for name in rows {
            names.push(name?);
        }
        Ok(names)
    }

    /// Record a message
    pub fn add_message(
        &self,
        room: &str,
        sender_type: &str,
        sender_name: &str,
        content_type: &str,
        content: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO messages (room, timestamp, sender_type, sender_name, content_type, content) \
             VALUES (?1, datetime('now'), ?2, ?3, ?4, ?5)",
            params![room, sender_type, sender_name, content_type, content],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get recent messages for a room
    pub fn recent_messages(&self, room: &str, limit: usize) -> Result<Vec<MessageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, sender_type, sender_name, content_type, content \
             FROM messages WHERE room = ?1 ORDER BY id DESC LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![room, limit], |row| {
            Ok(MessageRow {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                sender_type: row.get(2)?,
                sender_name: row.get(3)?,
                content_type: row.get(4)?,
                content: row.get(5)?,
            })
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        messages.reverse(); // Oldest first
        Ok(messages)
    }

    /// Record an artifact
    pub fn add_artifact(
        &self,
        id: &str,
        room: &str,
        artifact_type: &str,
        name: &str,
        created_by: &str,
        cas_hash: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO artifacts (id, room, artifact_type, name, created_by, created_at, cas_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), ?6)",
            params![id, room, artifact_type, name, created_by, cas_hash],
        )?;
        Ok(())
    }
}

/// A message row from the database
#[derive(Debug)]
pub struct MessageRow {
    pub id: i64,
    pub timestamp: String,
    pub sender_type: String,
    pub sender_name: String,
    pub content_type: String,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database() -> Result<()> {
        let db = Database::in_memory()?;

        db.create_room("test", Some("A test room"))?;
        let rooms = db.list_rooms()?;
        assert_eq!(rooms, vec!["test"]);

        db.add_message("test", "user", "amy", "chat", "hello world")?;
        let messages = db.recent_messages("test", 10)?;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello world");

        Ok(())
    }
}
