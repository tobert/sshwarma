//! Persistence: sqlite for sessions, history, room state

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

use crate::display::{EntryContent, EntryId, EntrySource, LedgerEntry, PresenceAction};
use crate::world::{AssetBinding, Inspiration, JournalEntry, JournalKind};

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

            -- Room context (extends rooms table)
            CREATE TABLE IF NOT EXISTS room_context (
                room TEXT PRIMARY KEY,
                vibe TEXT,
                parent TEXT,
                FOREIGN KEY (room) REFERENCES rooms(name),
                FOREIGN KEY (parent) REFERENCES rooms(name)
            );

            CREATE TABLE IF NOT EXISTS journal_entries (
                id TEXT PRIMARY KEY,
                room TEXT NOT NULL,
                author TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                content TEXT NOT NULL,
                kind TEXT NOT NULL,
                FOREIGN KEY (room) REFERENCES rooms(name)
            );
            CREATE INDEX IF NOT EXISTS idx_journal_room ON journal_entries(room);
            CREATE INDEX IF NOT EXISTS idx_journal_kind ON journal_entries(kind);

            CREATE TABLE IF NOT EXISTS asset_bindings (
                room TEXT NOT NULL,
                role TEXT NOT NULL,
                artifact_id TEXT NOT NULL,
                notes TEXT,
                bound_by TEXT NOT NULL,
                bound_at TEXT NOT NULL,
                PRIMARY KEY (room, role),
                FOREIGN KEY (room) REFERENCES rooms(name)
            );

            CREATE TABLE IF NOT EXISTS room_exits (
                from_room TEXT NOT NULL,
                direction TEXT NOT NULL,
                to_room TEXT NOT NULL,
                PRIMARY KEY (from_room, direction),
                FOREIGN KEY (from_room) REFERENCES rooms(name),
                FOREIGN KEY (to_room) REFERENCES rooms(name)
            );

            CREATE TABLE IF NOT EXISTS room_tags (
                room TEXT NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (room, tag),
                FOREIGN KEY (room) REFERENCES rooms(name)
            );

            CREATE TABLE IF NOT EXISTS room_inspirations (
                id TEXT PRIMARY KEY,
                room TEXT NOT NULL,
                content TEXT NOT NULL,
                added_by TEXT NOT NULL,
                added_at TEXT NOT NULL,
                FOREIGN KEY (room) REFERENCES rooms(name)
            );

            -- Ledger entries (replaces messages table)
            CREATE TABLE IF NOT EXISTS ledger_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                room TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                -- EntrySource fields
                source_type TEXT NOT NULL,
                source_name TEXT,
                source_streaming INTEGER,
                -- EntryContent fields
                content_type TEXT NOT NULL,
                content TEXT NOT NULL,
                content_meta TEXT,
                -- Entry flags
                ephemeral INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (room) REFERENCES rooms(name)
            );

            CREATE INDEX IF NOT EXISTS idx_ledger_room ON ledger_entries(room);
            CREATE INDEX IF NOT EXISTS idx_ledger_timestamp ON ledger_entries(timestamp);
            "#,
        )?;

        // Migrations for existing databases
        self.run_migrations()?;

        Ok(())
    }

    fn run_migrations(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Add enable_navigation column to room_context (default enabled)
        // This is idempotent - fails silently if column already exists
        let _ = conn.execute(
            "ALTER TABLE room_context ADD COLUMN enable_navigation INTEGER DEFAULT 1",
            [],
        );

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

    /// Get recent messages for a room (legacy - use recent_entries instead)
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

    // =========================================================================
    // Ledger Entry Management (new)
    // =========================================================================

    /// Add a ledger entry to the database
    pub fn add_ledger_entry(&self, room: &str, entry: &LedgerEntry) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let timestamp = entry.timestamp.to_rfc3339();

        // Convert EntrySource to DB fields
        let (source_type, source_name, source_streaming) = match &entry.source {
            EntrySource::User(name) => ("user", Some(name.clone()), None),
            EntrySource::Model { name, is_streaming } => {
                ("model", Some(name.clone()), Some(if *is_streaming { 1 } else { 0 }))
            }
            EntrySource::System => ("system", None, None),
            EntrySource::Command { command } => ("command", Some(command.clone()), None),
        };

        // Convert EntryContent to DB fields
        let (content_type, content, content_meta) = match &entry.content {
            EntryContent::Chat(text) => ("chat", text.clone(), None),
            EntryContent::CommandOutput(text) => ("command_output", text.clone(), None),
            EntryContent::Status(_) => {
                // Status entries are transient, don't persist them
                return Ok(0);
            }
            EntryContent::RoomHeader { name, description } => {
                let meta = serde_json::json!({ "description": description });
                ("room_header", name.clone(), Some(meta.to_string()))
            }
            EntryContent::Welcome { username } => ("welcome", username.clone(), None),
            EntryContent::HistorySeparator { label } => ("history_separator", label.clone(), None),
            EntryContent::Error(msg) => ("error", msg.clone(), None),
            EntryContent::Presence { user, action } => {
                let action_str = match action {
                    PresenceAction::Join => "join",
                    PresenceAction::Leave => "leave",
                };
                let meta = serde_json::json!({ "action": action_str });
                ("presence", user.clone(), Some(meta.to_string()))
            }
            EntryContent::Compaction(summary) => ("compaction", summary.clone(), None),
        };

        conn.execute(
            "INSERT INTO ledger_entries \
             (room, timestamp, source_type, source_name, source_streaming, \
              content_type, content, content_meta, ephemeral) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                room,
                timestamp,
                source_type,
                source_name,
                source_streaming,
                content_type,
                content,
                content_meta,
                if entry.ephemeral { 1 } else { 0 }
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Get recent ledger entries for a room
    pub fn recent_entries(&self, room: &str, limit: usize) -> Result<Vec<LedgerEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, source_type, source_name, source_streaming, \
             content_type, content, content_meta, ephemeral \
             FROM ledger_entries WHERE room = ?1 ORDER BY id DESC LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![room, limit], |row| {
            let id: i64 = row.get(0)?;
            let timestamp_str: String = row.get(1)?;
            let source_type: String = row.get(2)?;
            let source_name: Option<String> = row.get(3)?;
            let source_streaming: Option<i32> = row.get(4)?;
            let content_type: String = row.get(5)?;
            let content: String = row.get(6)?;
            let content_meta: Option<String> = row.get(7)?;
            let ephemeral: i32 = row.get(8)?;

            // Parse EntrySource
            let source = match source_type.as_str() {
                "user" => EntrySource::User(source_name.unwrap_or_default()),
                "model" => EntrySource::Model {
                    name: source_name.unwrap_or_default(),
                    is_streaming: source_streaming.unwrap_or(0) == 1,
                },
                "system" => EntrySource::System,
                "command" => EntrySource::Command {
                    command: source_name.unwrap_or_default(),
                },
                _ => EntrySource::System,
            };

            // Parse EntryContent
            let entry_content = match content_type.as_str() {
                "chat" => EntryContent::Chat(content),
                "command_output" => EntryContent::CommandOutput(content),
                "room_header" => {
                    let desc = content_meta
                        .as_ref()
                        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
                        .and_then(|v| v.get("description").and_then(|d| d.as_str()).map(String::from));
                    EntryContent::RoomHeader {
                        name: content,
                        description: desc,
                    }
                }
                "welcome" => EntryContent::Welcome { username: content },
                "history_separator" => EntryContent::HistorySeparator { label: content },
                "error" => EntryContent::Error(content),
                "presence" => {
                    let action = content_meta
                        .as_ref()
                        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
                        .and_then(|v| v.get("action").and_then(|a| a.as_str()).map(String::from))
                        .unwrap_or_else(|| "join".to_string());
                    EntryContent::Presence {
                        user: content,
                        action: if action == "leave" {
                            PresenceAction::Leave
                        } else {
                            PresenceAction::Join
                        },
                    }
                }
                "compaction" => EntryContent::Compaction(content),
                _ => EntryContent::Chat(content),
            };

            Ok(LedgerEntry {
                id: EntryId(id as u64),
                timestamp: chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                source,
                content: entry_content,
                mutable: false, // DB entries are never mutable
                ephemeral: ephemeral == 1,
                collapsible: true,
            })
        })?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        entries.reverse(); // Oldest first
        Ok(entries)
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

    // =========================================================================
    // Room Context
    // =========================================================================

    /// Set the vibe for a room
    pub fn set_vibe(&self, room: &str, vibe: Option<&str>) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO room_context (room, vibe) VALUES (?1, ?2) \
             ON CONFLICT(room) DO UPDATE SET vibe = ?2",
            params![room, vibe],
        )?;
        Ok(())
    }

    /// Get the vibe for a room
    pub fn get_vibe(&self, room: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT vibe FROM room_context WHERE room = ?1")?;
        let mut rows = stmt.query(params![room])?;
        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Ok(None)
        }
    }

    /// Get the parent room (for fork DAG)
    pub fn get_parent(&self, room: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT parent FROM room_context WHERE room = ?1")?;
        let mut rows = stmt.query(params![room])?;
        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Ok(None)
        }
    }

    /// Set the parent room (for fork DAG)
    pub fn set_parent(&self, room: &str, parent: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO room_context (room, parent) VALUES (?1, ?2) \
             ON CONFLICT(room) DO UPDATE SET parent = ?2",
            params![room, parent],
        )?;
        Ok(())
    }

    /// Get navigation enabled for a room (defaults to true)
    pub fn get_room_navigation(&self, room: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT enable_navigation FROM room_context WHERE room = ?1")?;
        let mut rows = stmt.query(params![room])?;
        if let Some(row) = rows.next()? {
            let enabled: Option<i32> = row.get(0)?;
            Ok(enabled.unwrap_or(1) == 1)
        } else {
            Ok(true) // Default enabled
        }
    }

    /// Set navigation enabled for a room
    pub fn set_room_navigation(&self, room: &str, enabled: bool) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO room_context (room, enable_navigation) VALUES (?1, ?2) \
             ON CONFLICT(room) DO UPDATE SET enable_navigation = ?2",
            params![room, if enabled { 1 } else { 0 }],
        )?;
        Ok(())
    }

    // =========================================================================
    // Journal Entries
    // =========================================================================

    /// Add a journal entry
    pub fn add_journal_entry(
        &self,
        room: &str,
        author: &str,
        content: &str,
        kind: JournalKind,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let timestamp = Utc::now().to_rfc3339();
        self.conn.lock().unwrap().execute(
            "INSERT INTO journal_entries (id, room, author, timestamp, content, kind) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, room, author, timestamp, content, kind.as_str()],
        )?;
        Ok(id)
    }

    /// Get journal entries for a room
    pub fn get_journal_entries(
        &self,
        room: &str,
        kind: Option<JournalKind>,
        limit: usize,
    ) -> Result<Vec<JournalEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut entries = Vec::new();

        if let Some(k) = kind {
            let mut stmt = conn.prepare(
                "SELECT id, timestamp, author, content, kind FROM journal_entries \
                 WHERE room = ?1 AND kind = ?2 ORDER BY timestamp DESC LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![room, k.as_str(), limit], |row| {
                let kind_str: String = row.get(4)?;
                Ok(JournalEntry {
                    id: row.get(0)?,
                    timestamp: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    author: row.get(2)?,
                    content: row.get(3)?,
                    kind: JournalKind::from_str(&kind_str).unwrap_or(JournalKind::Note),
                })
            })?;
            for row in rows {
                entries.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, timestamp, author, content, kind FROM journal_entries \
                 WHERE room = ?1 ORDER BY timestamp DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![room, limit], |row| {
                let kind_str: String = row.get(4)?;
                Ok(JournalEntry {
                    id: row.get(0)?,
                    timestamp: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    author: row.get(2)?,
                    content: row.get(3)?,
                    kind: JournalKind::from_str(&kind_str).unwrap_or(JournalKind::Note),
                })
            })?;
            for row in rows {
                entries.push(row?);
            }
        }

        entries.reverse(); // Oldest first
        Ok(entries)
    }

    // =========================================================================
    // Asset Bindings
    // =========================================================================

    /// Bind an artifact to a room with a semantic role
    pub fn bind_asset(
        &self,
        room: &str,
        role: &str,
        artifact_id: &str,
        notes: Option<&str>,
        bound_by: &str,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        self.conn.lock().unwrap().execute(
            "INSERT INTO asset_bindings (room, role, artifact_id, notes, bound_by, bound_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(room, role) DO UPDATE SET artifact_id = ?3, notes = ?4, bound_by = ?5, bound_at = ?6",
            params![room, role, artifact_id, notes, bound_by, timestamp],
        )?;
        Ok(())
    }

    /// Unbind an asset from a room
    pub fn unbind_asset(&self, room: &str, role: &str) -> Result<bool> {
        let count = self.conn.lock().unwrap().execute(
            "DELETE FROM asset_bindings WHERE room = ?1 AND role = ?2",
            params![room, role],
        )?;
        Ok(count > 0)
    }

    /// Get a specific asset binding
    pub fn get_asset_binding(&self, room: &str, role: &str) -> Result<Option<AssetBinding>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT artifact_id, role, notes, bound_by, bound_at FROM asset_bindings \
             WHERE room = ?1 AND role = ?2",
        )?;
        let mut rows = stmt.query(params![room, role])?;
        if let Some(row) = rows.next()? {
            Ok(Some(AssetBinding {
                artifact_id: row.get(0)?,
                role: row.get(1)?,
                notes: row.get(2)?,
                bound_by: row.get(3)?,
                bound_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            }))
        } else {
            Ok(None)
        }
    }

    /// List all asset bindings for a room
    pub fn list_asset_bindings(&self, room: &str) -> Result<Vec<AssetBinding>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT artifact_id, role, notes, bound_by, bound_at FROM asset_bindings \
             WHERE room = ?1 ORDER BY role",
        )?;
        let rows = stmt.query_map(params![room], |row| {
            Ok(AssetBinding {
                artifact_id: row.get(0)?,
                role: row.get(1)?,
                notes: row.get(2)?,
                bound_by: row.get(3)?,
                bound_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?;
        let mut bindings = Vec::new();
        for row in rows {
            bindings.push(row?);
        }
        Ok(bindings)
    }

    // =========================================================================
    // Room Exits
    // =========================================================================

    /// Add an exit from one room to another
    pub fn add_exit(&self, from_room: &str, direction: &str, to_room: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO room_exits (from_room, direction, to_room) VALUES (?1, ?2, ?3) \
             ON CONFLICT(from_room, direction) DO UPDATE SET to_room = ?3",
            params![from_room, direction, to_room],
        )?;
        Ok(())
    }

    /// Remove an exit
    pub fn remove_exit(&self, from_room: &str, direction: &str) -> Result<bool> {
        let count = self.conn.lock().unwrap().execute(
            "DELETE FROM room_exits WHERE from_room = ?1 AND direction = ?2",
            params![from_room, direction],
        )?;
        Ok(count > 0)
    }

    /// Get all exits from a room
    pub fn get_exits(&self, room: &str) -> Result<HashMap<String, String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT direction, to_room FROM room_exits WHERE from_room = ?1",
        )?;
        let rows = stmt.query_map(params![room], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut exits = HashMap::new();
        for row in rows {
            let (dir, target) = row?;
            exits.insert(dir, target);
        }
        Ok(exits)
    }

    // =========================================================================
    // Room Tags
    // =========================================================================

    /// Add a tag to a room
    pub fn add_tag(&self, room: &str, tag: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT OR IGNORE INTO room_tags (room, tag) VALUES (?1, ?2)",
            params![room, tag],
        )?;
        Ok(())
    }

    /// Remove a tag from a room
    pub fn remove_tag(&self, room: &str, tag: &str) -> Result<bool> {
        let count = self.conn.lock().unwrap().execute(
            "DELETE FROM room_tags WHERE room = ?1 AND tag = ?2",
            params![room, tag],
        )?;
        Ok(count > 0)
    }

    /// Get all tags for a room
    pub fn get_tags(&self, room: &str) -> Result<HashSet<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT tag FROM room_tags WHERE room = ?1")?;
        let rows = stmt.query_map(params![room], |row| row.get(0))?;
        let mut tags = HashSet::new();
        for row in rows {
            tags.insert(row?);
        }
        Ok(tags)
    }

    // =========================================================================
    // Room Inspirations
    // =========================================================================

    /// Add an inspiration to a room
    pub fn add_inspiration(&self, room: &str, content: &str, added_by: &str) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let timestamp = Utc::now().to_rfc3339();
        self.conn.lock().unwrap().execute(
            "INSERT INTO room_inspirations (id, room, content, added_by, added_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, room, content, added_by, timestamp],
        )?;
        Ok(id)
    }

    /// Get all inspirations for a room
    pub fn get_inspirations(&self, room: &str) -> Result<Vec<Inspiration>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, content, added_by, added_at FROM room_inspirations \
             WHERE room = ?1 ORDER BY added_at",
        )?;
        let rows = stmt.query_map(params![room], |row| {
            Ok(Inspiration {
                id: row.get(0)?,
                content: row.get(1)?,
                added_by: row.get(2)?,
                added_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(3)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?;
        let mut inspirations = Vec::new();
        for row in rows {
            inspirations.push(row?);
        }
        Ok(inspirations)
    }

    // =========================================================================
    // Fork DAG
    // =========================================================================

    /// Fork a room, creating a child with inherited context
    pub fn fork_room(&self, source: &str, new_name: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        // Create the new room
        conn.execute(
            "INSERT INTO rooms (name, description, created_at) \
             SELECT ?1, description, ?2 FROM rooms WHERE name = ?3",
            params![new_name, now, source],
        )?;

        // Copy vibe, enable_navigation and set parent
        conn.execute(
            "INSERT INTO room_context (room, vibe, parent, enable_navigation) \
             SELECT ?1, vibe, ?2, enable_navigation FROM room_context WHERE room = ?2",
            params![new_name, source],
        )?;

        // If source had no context, create one with just parent
        conn.execute(
            "INSERT OR IGNORE INTO room_context (room, parent) VALUES (?1, ?2)",
            params![new_name, source],
        )?;

        // Copy tags
        conn.execute(
            "INSERT INTO room_tags (room, tag) \
             SELECT ?1, tag FROM room_tags WHERE room = ?2",
            params![new_name, source],
        )?;

        // Copy asset bindings with fresh timestamps
        conn.execute(
            "INSERT INTO asset_bindings (room, role, artifact_id, notes, bound_by, bound_at) \
             SELECT ?1, role, artifact_id, notes, bound_by, ?2 FROM asset_bindings WHERE room = ?3",
            params![new_name, now, source],
        )?;

        // Copy inspirations
        conn.execute(
            "INSERT INTO room_inspirations (id, room, content, added_by, added_at) \
             SELECT ?1 || '-' || id, ?2, content, added_by, added_at \
             FROM room_inspirations WHERE room = ?3",
            params![Uuid::new_v4().to_string(), new_name, source],
        )?;

        Ok(())
    }

    /// Get the ancestry chain for a room (parent, grandparent, etc.)
    pub fn get_ancestry(&self, room: &str) -> Result<Vec<String>> {
        let mut ancestry = Vec::new();
        let mut current = room.to_string();

        loop {
            if let Some(parent) = self.get_parent(&current)? {
                ancestry.push(parent.clone());
                current = parent;
            } else {
                break;
            }
        }

        Ok(ancestry)
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
