//! Persistence: sqlite for sessions, history, room state

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

/// Database handle (thread-safe via Mutex)
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open or create database at path
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn: Mutex::new(conn) };
        db.init_schema()?;
        Ok(db)
    }

    /// Open in-memory database (for testing)
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn: Mutex::new(conn) };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.lock().unwrap().execute_batch(
            r#"
            -- Users and their SSH public keys
            CREATE TABLE IF NOT EXISTS users (
                handle TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                last_seen TEXT
            );

            CREATE TABLE IF NOT EXISTS pubkeys (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                handle TEXT NOT NULL,
                pubkey TEXT NOT NULL UNIQUE,
                key_type TEXT NOT NULL,
                comment TEXT,
                added_at TEXT NOT NULL,
                FOREIGN KEY (handle) REFERENCES users(handle)
            );

            CREATE INDEX IF NOT EXISTS idx_pubkeys_handle ON pubkeys(handle);

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

    // =========================================================================
    // User / Pubkey Management
    // =========================================================================

    /// Add a user (creates if not exists)
    pub fn add_user(&self, handle: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT OR IGNORE INTO users (handle, created_at) VALUES (?1, datetime('now'))",
            params![handle],
        )?;
        Ok(())
    }

    /// Add a public key for a user
    pub fn add_pubkey(
        &self,
        handle: &str,
        pubkey: &str,
        key_type: &str,
        comment: Option<&str>,
    ) -> Result<()> {
        // Ensure user exists
        self.add_user(handle)?;

        self.conn.lock().unwrap().execute(
            "INSERT INTO pubkeys (handle, pubkey, key_type, comment, added_at) \
             VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![handle, pubkey, key_type, comment],
        )?;
        Ok(())
    }

    /// Remove a public key
    pub fn remove_pubkey(&self, pubkey: &str) -> Result<bool> {
        let count = self.conn.lock().unwrap().execute("DELETE FROM pubkeys WHERE pubkey = ?1", params![pubkey])?;
        Ok(count > 0)
    }

    /// Remove all keys for a user
    pub fn remove_user(&self, handle: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM pubkeys WHERE handle = ?1", params![handle])?;
        let count = conn.execute("DELETE FROM users WHERE handle = ?1", params![handle])?;
        Ok(count > 0)
    }

    /// Look up handle by public key (for auth)
    pub fn lookup_handle_by_pubkey(&self, pubkey: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT handle FROM pubkeys WHERE pubkey = ?1")?;
        let mut rows = stmt.query(params![pubkey])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Update last_seen for a user
    pub fn touch_user(&self, handle: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE users SET last_seen = datetime('now') WHERE handle = ?1",
            params![handle],
        )?;
        Ok(())
    }

    /// List all users
    pub fn list_users(&self) -> Result<Vec<UserInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT u.handle, u.created_at, u.last_seen, COUNT(p.id) as key_count \
             FROM users u LEFT JOIN pubkeys p ON u.handle = p.handle \
             GROUP BY u.handle ORDER BY u.handle",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(UserInfo {
                handle: row.get(0)?,
                created_at: row.get(1)?,
                last_seen: row.get(2)?,
                key_count: row.get(3)?,
            })
        })?;
        let mut users = Vec::new();
        for row in rows {
            users.push(row?);
        }
        Ok(users)
    }

    /// List keys for a user
    pub fn list_keys_for_user(&self, handle: &str) -> Result<Vec<PubkeyInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, pubkey, key_type, comment, added_at \
             FROM pubkeys WHERE handle = ?1 ORDER BY added_at",
        )?;
        let rows = stmt.query_map(params![handle], |row| {
            Ok(PubkeyInfo {
                id: row.get(0)?,
                pubkey: row.get(1)?,
                key_type: row.get(2)?,
                comment: row.get(3)?,
                added_at: row.get(4)?,
            })
        })?;
        let mut keys = Vec::new();
        for row in rows {
            keys.push(row?);
        }
        Ok(keys)
    }

    // =========================================================================
    // Room Management
    // =========================================================================

    /// Create a room
    pub fn create_room(&self, name: &str, description: Option<&str>) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO rooms (name, description, created_at) VALUES (?1, ?2, datetime('now'))",
            params![name, description],
        )?;
        Ok(())
    }

    /// Get room names
    pub fn list_rooms(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT name FROM rooms ORDER BY name")?;
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
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (room, timestamp, sender_type, sender_name, content_type, content) \
             VALUES (?1, datetime('now'), ?2, ?3, ?4, ?5)",
            params![room, sender_type, sender_name, content_type, content],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get recent messages for a room
    pub fn recent_messages(&self, room: &str, limit: usize) -> Result<Vec<MessageRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
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
        self.conn.lock().unwrap().execute(
            "INSERT INTO artifacts (id, room, artifact_type, name, created_by, created_at, cas_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), ?6)",
            params![id, room, artifact_type, name, created_by, cas_hash],
        )?;
        Ok(())
    }

    // =========================================================================
    // Session Management
    // =========================================================================

    /// Start a new session
    pub fn start_session(&self, session_id: &str, username: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO sessions (id, username, connected_at) VALUES (?1, ?2, datetime('now'))",
            params![session_id, username],
        )?;
        Ok(())
    }

    /// End a session
    pub fn end_session(&self, session_id: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE sessions SET disconnected_at = datetime('now') WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Update session's current room
    pub fn update_session_room(&self, session_id: &str, room: Option<&str>) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE sessions SET current_room = ?1 WHERE id = ?2",
            params![room, session_id],
        )?;
        Ok(())
    }

    // =========================================================================
    // Room Loading
    // =========================================================================

    /// Get all rooms with their info
    pub fn get_all_rooms(&self) -> Result<Vec<RoomInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT name, description, created_at FROM rooms ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RoomInfo {
                name: row.get(0)?,
                description: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        let mut rooms = Vec::new();
        for row in rows {
            rooms.push(row?);
        }
        Ok(rooms)
    }

    /// Get room info by name
    pub fn get_room(&self, name: &str) -> Result<Option<RoomInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT name, description, created_at FROM rooms WHERE name = ?1",
        )?;
        let mut rows = stmt.query(params![name])?;
        if let Some(row) = rows.next()? {
            Ok(Some(RoomInfo {
                name: row.get(0)?,
                description: row.get(1)?,
                created_at: row.get(2)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Check if a room exists
    pub fn room_exists(&self, name: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM rooms WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

/// User info from the database
#[derive(Debug, Clone)]
pub struct UserInfo {
    pub handle: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    pub key_count: i64,
}

/// Public key info from the database
#[derive(Debug, Clone)]
pub struct PubkeyInfo {
    pub id: i64,
    pub pubkey: String,
    pub key_type: String,
    pub comment: Option<String>,
    pub added_at: String,
}

/// A message row from the database
#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: i64,
    pub timestamp: String,
    pub sender_type: String,
    pub sender_name: String,
    pub content_type: String,
    pub content: String,
}

/// Room info from the database
#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
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

    #[test]
    fn test_pubkey_management() -> Result<()> {
        let db = Database::in_memory()?;

        // Add a user with a key
        let pubkey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAItest test@example";
        db.add_pubkey("amy", pubkey, "ssh-ed25519", Some("test key"))?;

        // Should find the user by key
        let handle = db.lookup_handle_by_pubkey(pubkey)?;
        assert_eq!(handle, Some("amy".to_string()));

        // Unknown key returns None
        let unknown = db.lookup_handle_by_pubkey("ssh-ed25519 unknown")?;
        assert_eq!(unknown, None);

        // List users
        let users = db.list_users()?;
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].handle, "amy");
        assert_eq!(users[0].key_count, 1);

        // Add another key for same user
        let pubkey2 = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAItest2 amy@laptop";
        db.add_pubkey("amy", pubkey2, "ssh-ed25519", Some("laptop key"))?;

        let keys = db.list_keys_for_user("amy")?;
        assert_eq!(keys.len(), 2);

        // Remove a key
        db.remove_pubkey(pubkey)?;
        let keys = db.list_keys_for_user("amy")?;
        assert_eq!(keys.len(), 1);

        // Remove user entirely
        db.remove_user("amy")?;
        let users = db.list_users()?;
        assert_eq!(users.len(), 0);

        Ok(())
    }
}
