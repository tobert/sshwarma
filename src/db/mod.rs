//! Database module for sshwarma
//!
//! Provides persistence for agents, rooms, buffers, rows, and UI state.
//! Uses SQLite with UUIDv7 for primary keys and fractional indexing for ordering.

mod schema;

pub mod agents;
pub mod buffers;
pub mod rooms;
pub mod rows;
pub mod rules;
pub mod scripts;
pub mod view;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub use schema::{PRESENCE_QUERY, ROW_DEPTH_CTE, SCHEMA, SCHEMA_VERSION};

/// Generate a new UUIDv7 (time-sorted)
pub fn new_id() -> String {
    Uuid::now_v7().to_string()
}

/// Get current Unix timestamp in milliseconds
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

/// Database handle (thread-safe via Mutex)
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Acquire the database connection, converting PoisonError to anyhow::Error.
    pub(crate) fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| anyhow::anyhow!("database lock poisoned: {}", e))
    }

    /// Open or create database at path
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path.as_ref())
            .with_context(|| format!("failed to open database at {:?}", path.as_ref()))?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init()?;
        Ok(db)
    }

    /// Open in-memory database (for testing)
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("failed to open in-memory database")?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init()?;
        Ok(db)
    }

    /// Initialize schema and run migrations
    fn init(&self) -> Result<()> {
        let version = self.get_schema_version()?;

        if version < SCHEMA_VERSION {
            // Fresh install or major upgrade - apply full schema
            self.conn()?
                .execute_batch(SCHEMA)
                .context("failed to create schema")?;
            self.set_schema_version(SCHEMA_VERSION)?;
            tracing::info!(
                "initialized database schema version {}",
                SCHEMA_VERSION
            );
        }

        Ok(())
    }

    /// Get current schema version from user_version pragma
    fn get_schema_version(&self) -> Result<i32> {
        let conn = self.conn()?;
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .context("failed to get schema version")?;
        Ok(version)
    }

    /// Set schema version using user_version pragma
    fn set_schema_version(&self, version: i32) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(&format!("PRAGMA user_version = {}", version), [])
            .context("failed to set schema version")?;
        Ok(())
    }
}

// =============================================================================
// COMPATIBILITY STUBS - TO BE REMOVED IN TASK 09 (REWRITE CALLERS)
// =============================================================================
// These stubs allow the codebase to compile while we transition to the new
// data model. They will panic if called, indicating callers need rewriting.

/// Legacy message row type - replaced by Row
#[derive(Debug, Clone, Default)]
pub struct MessageRow {
    pub id: i64,
    pub room: String,
    pub sender: String,
    pub sender_name: String,
    pub sender_type: String,
    pub target: Option<String>,
    pub content: String,
    pub message_type: String,
    pub hidden: bool,
    pub created_at: String,
    pub timestamp: String,
}

/// Legacy journal entry type - replaced by Row with tags
#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub id: i64,
    pub room: String,
    pub kind: crate::world::JournalKind,
    pub content: String,
    pub author: String,
    pub created_at: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Legacy asset binding type - replaced by room_kv
#[derive(Debug, Clone)]
pub struct AssetBinding {
    pub room: String,
    pub role: String,
    pub artifact_id: String,
    pub notes: Option<String>,
    pub bound_by: String,
    pub bound_at: chrono::DateTime<chrono::Utc>,
}

/// Legacy room prompt type - to be replaced
#[derive(Debug, Clone)]
pub struct RoomPrompt {
    pub id: i64,
    pub room: String,
    pub name: String,
    pub prompt: String,
    pub content: String, // Alias for prompt
    pub created_at: String,
    pub created_by: Option<String>,
}

/// Legacy inspiration type
#[derive(Debug, Clone)]
pub struct Inspiration {
    pub content: String,
}

/// Legacy target with slots
#[derive(Debug, Clone)]
pub struct TargetWithSlots {
    pub target: String,
    pub target_type: String,
    pub slots: Vec<PromptSlot>,
}

/// Legacy prompt slot
#[derive(Debug, Clone)]
pub struct PromptSlot {
    pub index: usize,
    pub prompt_name: String,
    pub content: Option<String>,
    pub target_type: String,
}

/// Legacy user info
#[derive(Debug, Clone)]
pub struct UserInfo {
    pub handle: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    pub key_count: usize,
}

/// Legacy pubkey info
#[derive(Debug, Clone)]
pub struct PubkeyInfo {
    pub key: String,
    pub pubkey: String, // Alias for key
    pub key_type: String,
    pub comment: Option<String>,
    pub created_at: String,
}

/// Legacy room info
#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub name: String,
    pub vibe: Option<String>,
    pub description: Option<String>,
    pub created_at: String,
}

impl Database {
    // --- Legacy stubs that panic - callers need rewriting ---

    #[allow(unused_variables)]
    pub fn lookup_handle_by_pubkey(&self, key: &str) -> Result<Option<String>> {
        todo!("REWRITE CALLER: lookup_handle_by_pubkey -> use find_agent_by_auth(AuthKind::Pubkey, key)")
    }

    #[allow(unused_variables)]
    pub fn touch_user(&self, handle: &str) -> Result<()> {
        // This can be a no-op for now - we don't track "last seen" in the new model
        Ok(())
    }

    #[allow(unused_variables)]
    pub fn start_session(&self, session_id: &str, handle: &str) -> Result<()> {
        todo!("REWRITE CALLER: start_session -> use insert_session with AgentSession")
    }

    #[allow(unused_variables)]
    pub fn update_session_room(&self, session_id: &str, room: Option<&str>) -> Result<()> {
        // Sessions don't track room in new model - room membership is via presence rows
        Ok(())
    }

    #[allow(unused_variables)]
    pub fn add_ledger_entry(&self, room: &str, entry: &crate::display::LedgerEntry) -> Result<()> {
        todo!("REWRITE CALLER: add_ledger_entry -> use append_row with Row")
    }

    #[allow(unused_variables)]
    pub fn recent_messages(&self, room: &str, limit: usize) -> Result<Vec<MessageRow>> {
        todo!("REWRITE CALLER: recent_messages -> use list_buffer_rows")
    }

    #[allow(unused_variables)]
    pub fn get_vibe(&self, room: &str) -> Result<Option<String>> {
        todo!("REWRITE CALLER: get_vibe -> use get_room_kv(room_id, \"vibe\")")
    }

    #[allow(unused_variables)]
    pub fn set_vibe(&self, room: &str, vibe: Option<&str>) -> Result<()> {
        todo!("REWRITE CALLER: set_vibe -> use set_room_kv(room_id, \"vibe\", vibe)")
    }

    #[allow(unused_variables)]
    pub fn add_journal_entry(&self, room: &str, author: &str, content: &str, kind: crate::world::JournalKind) -> Result<()> {
        todo!("REWRITE CALLER: add_journal_entry -> use append_row with Row and add_row_tag")
    }

    #[allow(unused_variables)]
    pub fn get_journal_entries(&self, room: &str, kind: Option<crate::world::JournalKind>, limit: usize) -> Result<Vec<JournalEntry>> {
        todo!("REWRITE CALLER: get_journal_entries -> use find_rows_by_tag or list_rows_by_method")
    }

    #[allow(unused_variables)]
    pub fn get_asset_binding(&self, room: &str, role: &str) -> Result<Option<AssetBinding>> {
        todo!("REWRITE CALLER: get_asset_binding -> use get_room_kv with asset.{role} key")
    }

    #[allow(unused_variables)]
    pub fn bind_asset(&self, room: &str, role: &str, artifact_id: &str, notes: Option<&str>, bound_by: &str) -> Result<()> {
        todo!("REWRITE CALLER: bind_asset -> use set_room_kv")
    }

    #[allow(unused_variables)]
    pub fn unbind_asset(&self, room: &str, role: &str) -> Result<()> {
        todo!("REWRITE CALLER: unbind_asset -> use delete_room_kv")
    }

    #[allow(unused_variables)]
    pub fn get_inspirations(&self, room: &str) -> Result<Vec<Inspiration>> {
        todo!("REWRITE CALLER: get_inspirations -> use get_room_kv with inspiration.* pattern")
    }

    #[allow(unused_variables)]
    pub fn add_inspiration(&self, room: &str, content: &str, added_by: &str) -> Result<()> {
        todo!("REWRITE CALLER: add_inspiration -> use set_room_kv")
    }

    #[allow(unused_variables)]
    pub fn list_prompts(&self, room: &str) -> Result<Vec<RoomPrompt>> {
        todo!("REWRITE CALLER: list_prompts -> prompts are now scripts in lua_scripts table")
    }

    #[allow(unused_variables)]
    pub fn add_prompt(&self, room: &str, name: &str, prompt: &str) -> Result<()> {
        todo!("REWRITE CALLER: add_prompt -> use insert_script")
    }

    #[allow(unused_variables)]
    pub fn get_prompt(&self, room: &str, name: &str) -> Result<Option<RoomPrompt>> {
        todo!("REWRITE CALLER: get_prompt -> use get_script_by_name")
    }

    #[allow(unused_variables)]
    pub fn delete_prompt(&self, room: &str, name: &str) -> Result<bool> {
        todo!("REWRITE CALLER: delete_prompt -> use delete_script")
    }

    #[allow(unused_variables)]
    pub fn list_targets_with_slots(&self, room: &str) -> Result<Vec<(String, String)>> {
        todo!("REWRITE CALLER: list_targets_with_slots -> use room rules")
    }

    #[allow(unused_variables)]
    pub fn get_target_slots(&self, room: &str, target: &str) -> Result<Vec<PromptSlot>> {
        todo!("REWRITE CALLER: get_target_slots -> use room rules")
    }

    #[allow(unused_variables)]
    pub fn set_prompt(&self, room: &str, name: &str, content: &str, created_by: &str) -> Result<()> {
        todo!("REWRITE CALLER: set_prompt -> use insert_script")
    }

    #[allow(unused_variables)]
    pub fn push_slot(&self, room: &str, target: &str, target_type: &str, prompt_name: &str) -> Result<()> {
        todo!("REWRITE CALLER: push_slot -> use room rules")
    }

    #[allow(unused_variables)]
    pub fn pop_slot(&self, room: &str, target: &str) -> Result<bool> {
        todo!("REWRITE CALLER: pop_slot -> use room rules")
    }

    #[allow(unused_variables)]
    pub fn rm_slot(&self, room: &str, target: &str, index: i64) -> Result<bool> {
        todo!("REWRITE CALLER: rm_slot -> use room rules")
    }

    #[allow(unused_variables)]
    pub fn insert_slot(&self, room: &str, target: &str, target_type: &str, index: i64, prompt_name: &str) -> Result<()> {
        todo!("REWRITE CALLER: insert_slot -> use room rules")
    }

    #[allow(unused_variables)]
    pub fn ensure_room(&self, room: &str) -> Result<()> {
        // Check if room exists, create if not
        if self.get_room_by_name(room)?.is_none() {
            let room_obj = rooms::Room::new(room);
            self.insert_room(&room_obj)?;
        }
        Ok(())
    }

    #[allow(unused_variables)]
    pub fn get_rooms(&self) -> Result<Vec<String>> {
        let rooms = self.list_rooms()?;
        Ok(rooms.into_iter().map(|r| r.name).collect())
    }

    #[allow(unused_variables)]
    pub fn get_exits(&self, room: &str) -> Result<std::collections::HashMap<String, String>> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            self.get_room_exits(&room_obj.id)
        } else {
            Ok(std::collections::HashMap::new())
        }
    }

    #[allow(unused_variables)]
    pub fn add_exit(&self, room: &str, direction: &str, target: &str) -> Result<()> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            self.set_room_exit(&room_obj.id, direction, target)
        } else {
            Ok(())
        }
    }

    #[allow(unused_variables)]
    pub fn create_room(&self, room: &str, description: Option<&str>) -> Result<()> {
        let room_obj = rooms::Room::new(room);
        self.insert_room(&room_obj)?;
        if let Some(desc) = description {
            self.set_room_kv(&room_obj.id, "description", Some(desc))?;
        }
        Ok(())
    }

    #[allow(unused_variables)]
    pub fn fork_room(&self, source: &str, new_name: &str) -> Result<()> {
        todo!("REWRITE CALLER: fork_room -> copy room kv, create new room")
    }

    #[allow(unused_variables)]
    pub fn recent_entries(&self, room: &str, limit: usize) -> Result<Vec<crate::display::LedgerEntry>> {
        todo!("REWRITE CALLER: recent_entries -> use list_buffer_rows")
    }

    #[allow(unused_variables)]
    pub fn get_parent(&self, room: &str) -> Result<Option<String>> {
        todo!("REWRITE CALLER: get_parent -> use room kv")
    }

    #[allow(unused_variables)]
    pub fn get_tags(&self, room: &str) -> Result<Vec<String>> {
        todo!("REWRITE CALLER: get_tags -> use room kv")
    }

    #[allow(unused_variables)]
    pub fn list_asset_bindings(&self, room: &str) -> Result<Vec<AssetBinding>> {
        todo!("REWRITE CALLER: list_asset_bindings -> use room kv with asset.* pattern")
    }

    #[allow(unused_variables)]
    pub fn add_pubkey(&self, handle: &str, key: &str, key_type: &str, comment: Option<&str>) -> Result<()> {
        todo!("REWRITE CALLER: add_pubkey -> use upsert_auth with AuthKind::Pubkey")
    }

    #[allow(unused_variables)]
    pub fn remove_user(&self, handle: &str) -> Result<bool> {
        todo!("REWRITE CALLER: remove_user -> use delete_agent")
    }

    #[allow(unused_variables)]
    pub fn remove_pubkey(&self, key: &str) -> Result<bool> {
        todo!("REWRITE CALLER: remove_pubkey -> use delete_auth")
    }

    #[allow(unused_variables)]
    pub fn list_users(&self) -> Result<Vec<UserInfo>> {
        todo!("REWRITE CALLER: list_users -> use list_agents(Some(AgentKind::Human))")
    }

    #[allow(unused_variables)]
    pub fn list_keys_for_user(&self, handle: &str) -> Result<Vec<PubkeyInfo>> {
        todo!("REWRITE CALLER: list_keys_for_user -> query agent_auth")
    }

    #[allow(unused_variables)]
    pub fn get_all_rooms(&self) -> Result<Vec<RoomInfo>> {
        todo!("REWRITE CALLER: get_all_rooms -> use list_rooms")
    }

    #[allow(unused_variables)]
    pub fn get_room_navigation(&self, room: &str) -> Result<bool> {
        // Default to allowing navigation
        if let Some(room_obj) = self.get_room_by_name(room)? {
            if let Some(val) = self.get_room_kv(&room_obj.id, "navigation")? {
                return Ok(val != "false");
            }
        }
        Ok(true)
    }

    #[allow(unused_variables)]
    pub fn set_room_navigation(&self, room: &str, enabled: bool) -> Result<()> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            self.set_room_kv(&room_obj.id, "navigation", Some(if enabled { "true" } else { "false" }))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_id_is_valid_uuid() {
        let id = new_id();
        assert!(Uuid::parse_str(&id).is_ok());
    }

    #[test]
    fn test_new_id_is_sortable() {
        let id1 = new_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let id2 = new_id();
        // UUIDv7 is time-sorted, so id2 should be greater
        assert!(id2 > id1);
    }

    #[test]
    fn test_database_init() -> Result<()> {
        let db = Database::in_memory()?;
        let version = db.get_schema_version()?;
        assert_eq!(version, SCHEMA_VERSION);
        Ok(())
    }
}
