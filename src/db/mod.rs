//! Database module for sshwarma
//!
//! Provides persistence for agents, rooms, buffers, rows, and UI state.
//! Uses SQLite with UUIDv7 for primary keys and fractional indexing for ordering.

mod schema;

pub mod agents;
pub mod buffers;
pub mod equipped;
pub mod exits;
pub mod rooms;
pub mod rows;
pub mod scripts;
pub mod things;
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

/// Format a timestamp (milliseconds since epoch) as ISO 8601 string
fn format_timestamp(ms: i64) -> String {
    use chrono::{DateTime, Utc};
    let secs = ms / 1000;
    let nsecs = ((ms % 1000) * 1_000_000) as u32;
    DateTime::<Utc>::from_timestamp(secs, nsecs)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| format!("{}", ms))
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
            tracing::info!("initialized database schema version {}", SCHEMA_VERSION);
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

    /// Look up an agent handle (name) by SSH public key
    pub fn lookup_handle_by_pubkey(&self, key: &str) -> Result<Option<String>> {
        use agents::AuthKind;
        if let Some(agent) = self.find_agent_by_auth(AuthKind::Pubkey, key)? {
            Ok(Some(agent.name))
        } else {
            Ok(None)
        }
    }

    #[allow(unused_variables)]
    pub fn touch_user(&self, handle: &str) -> Result<()> {
        // This can be a no-op for now - we don't track "last seen" in the new model
        Ok(())
    }

    /// Start a new session for an agent
    pub fn start_session(&self, session_id: &str, handle: &str) -> Result<()> {
        use agents::{AgentSession, SessionKind};
        // Look up agent by name
        if let Some(agent) = self.get_agent_by_name(handle)? {
            // Create session with the provided ID
            let session = AgentSession {
                id: session_id.to_string(),
                agent_id: agent.id,
                kind: SessionKind::Ssh,
                connected_at: now_ms(),
                disconnected_at: None,
                metadata: None,
            };
            self.insert_session(&session)?;
        }
        Ok(())
    }

    #[allow(unused_variables)]
    pub fn update_session_room(&self, session_id: &str, room: Option<&str>) -> Result<()> {
        // Sessions don't track room in new model - room membership is via presence rows
        Ok(())
    }

    /// Get recent messages from a room's chat buffer
    pub fn recent_messages(&self, room: &str, limit: usize) -> Result<Vec<MessageRow>> {
        // Get room and buffer
        let room_obj = match self.get_room_by_name(room)? {
            Some(r) => r,
            None => return Ok(vec![]),
        };
        let buffer = self.get_or_create_room_chat_buffer(&room_obj.id)?;

        // Get recent rows
        let rows = self.list_recent_buffer_rows(&buffer.id, limit)?;

        // Convert to MessageRow
        let messages = rows
            .into_iter()
            .map(|row| {
                // Look up agent name if available
                let (sender, sender_name, sender_type) =
                    if let Some(agent_id) = &row.source_agent_id {
                        if let Ok(Some(agent)) = self.get_agent(agent_id) {
                            (
                                agent.name.clone(),
                                agent.display_name.unwrap_or(agent.name),
                                agent.kind.as_str().to_string(),
                            )
                        } else {
                            (
                                "unknown".to_string(),
                                "Unknown".to_string(),
                                "unknown".to_string(),
                            )
                        }
                    } else {
                        (
                            "system".to_string(),
                            "System".to_string(),
                            "system".to_string(),
                        )
                    };

                MessageRow {
                    id: 0, // Legacy, not used
                    room: room.to_string(),
                    sender,
                    sender_name,
                    sender_type,
                    target: None,
                    content: row.content.unwrap_or_default(),
                    message_type: row.content_method,
                    hidden: row.hidden,
                    created_at: format_timestamp(row.created_at),
                    timestamp: format_timestamp(row.created_at),
                }
            })
            .collect();

        Ok(messages)
    }

    /// Get room vibe
    pub fn get_vibe(&self, room: &str) -> Result<Option<String>> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            self.get_room_kv(&room_obj.id, "vibe")
        } else {
            Ok(None)
        }
    }

    /// Set room vibe
    pub fn set_vibe(&self, room: &str, vibe: Option<&str>) -> Result<()> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            self.set_room_kv(&room_obj.id, "vibe", vibe)?;
        }
        Ok(())
    }

    /// Get asset binding by role
    pub fn get_asset_binding(&self, room: &str, role: &str) -> Result<Option<AssetBinding>> {
        use chrono::{TimeZone, Utc};

        if let Some(room_obj) = self.get_room_by_name(room)? {
            let key = format!("asset.{}", role);
            if let Some(value) = self.get_room_kv(&room_obj.id, &key)? {
                // Parse JSON: {"artifact_id": "...", "notes": "...", "bound_by": "...", "bound_at": ms}
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&value) {
                    let bound_at_ms = json["bound_at"].as_i64().unwrap_or(0);
                    return Ok(Some(AssetBinding {
                        room: room.to_string(),
                        role: role.to_string(),
                        artifact_id: json["artifact_id"].as_str().unwrap_or("").to_string(),
                        notes: json["notes"].as_str().map(|s| s.to_string()),
                        bound_by: json["bound_by"].as_str().unwrap_or("").to_string(),
                        bound_at: Utc
                            .timestamp_millis_opt(bound_at_ms)
                            .single()
                            .unwrap_or_else(Utc::now),
                    }));
                }
            }
        }
        Ok(None)
    }

    /// Bind an asset to a room role
    pub fn bind_asset(
        &self,
        room: &str,
        role: &str,
        artifact_id: &str,
        notes: Option<&str>,
        bound_by: &str,
    ) -> Result<()> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            let key = format!("asset.{}", role);
            let json = serde_json::json!({
                "artifact_id": artifact_id,
                "notes": notes,
                "bound_by": bound_by,
                "bound_at": now_ms(),
            });
            self.set_room_kv(&room_obj.id, &key, Some(&json.to_string()))?;
        }
        Ok(())
    }

    /// Unbind an asset from a room role
    pub fn unbind_asset(&self, room: &str, role: &str) -> Result<()> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            let key = format!("asset.{}", role);
            self.delete_room_kv(&room_obj.id, &key)?;
        }
        Ok(())
    }

    /// List prompts for a room (stored as room-scoped scripts with "prompt." prefix)
    pub fn list_prompts(&self, room: &str) -> Result<Vec<RoomPrompt>> {
        use scripts::ScriptScope;

        let scripts = self.list_scripts(ScriptScope::Room, Some(room))?;

        let prompts = scripts
            .into_iter()
            .filter_map(|s| {
                let short_name = s.module_path.strip_prefix("prompt.")?;
                Some(RoomPrompt {
                    id: 0,
                    room: room.to_string(),
                    name: short_name.to_string(),
                    prompt: s.code.clone(),
                    content: s.code,
                    created_at: format_timestamp(s.created_at),
                    created_by: s.description,
                })
            })
            .collect();

        Ok(prompts)
    }

    /// Add a prompt to a room
    pub fn add_prompt(&self, room: &str, name: &str, prompt: &str) -> Result<()> {
        use scripts::ScriptScope;

        let module_path = format!("prompt.{}", name);
        self.create_script(
            ScriptScope::Room,
            Some(room),
            &module_path,
            prompt,
            "system",
        )?;
        Ok(())
    }

    /// Get a prompt from a room
    pub fn get_prompt(&self, room: &str, name: &str) -> Result<Option<RoomPrompt>> {
        use scripts::ScriptScope;

        let module_path = format!("prompt.{}", name);
        if let Some(script) =
            self.get_current_script(ScriptScope::Room, Some(room), &module_path)?
        {
            Ok(Some(RoomPrompt {
                id: 0,
                room: room.to_string(),
                name: name.to_string(),
                prompt: script.code.clone(),
                content: script.code,
                created_at: format_timestamp(script.created_at),
                created_by: script.description,
            }))
        } else {
            Ok(None)
        }
    }

    /// Delete a prompt from a room
    pub fn delete_prompt(&self, room: &str, name: &str) -> Result<bool> {
        use scripts::ScriptScope;

        let module_path = format!("prompt.{}", name);
        let deleted = self.delete_script(ScriptScope::Room, Some(room), &module_path)?;
        Ok(deleted > 0)
    }

    /// Set (create or update) a prompt in a room
    pub fn set_prompt(
        &self,
        room: &str,
        name: &str,
        content: &str,
        created_by: &str,
    ) -> Result<()> {
        use scripts::ScriptScope;

        let module_path = format!("prompt.{}", name);

        // Check if script exists
        if let Some(existing) =
            self.get_current_script(ScriptScope::Room, Some(room), &module_path)?
        {
            // Update existing script (CoW)
            self.update_script(&existing.id, content, created_by)?;
        } else {
            // Create new script
            self.create_script(
                ScriptScope::Room,
                Some(room),
                &module_path,
                content,
                created_by,
            )?;
        }
        Ok(())
    }

    pub fn ensure_room(&self, room: &str) -> Result<()> {
        // Check if room exists, create if not
        if self.get_room_by_name(room)?.is_none() {
            let room_obj = rooms::Room::new(room);
            self.insert_room(&room_obj)?;
        }
        Ok(())
    }

    pub fn get_rooms(&self) -> Result<Vec<String>> {
        let rooms = self.list_rooms()?;
        Ok(rooms.into_iter().map(|r| r.name).collect())
    }

    pub fn get_exits(&self, room: &str) -> Result<std::collections::HashMap<String, String>> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            self.get_room_exits(&room_obj.id)
        } else {
            Ok(std::collections::HashMap::new())
        }
    }

    pub fn add_exit(&self, room: &str, direction: &str, target: &str) -> Result<()> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            self.set_room_exit(&room_obj.id, direction, target)
        } else {
            Ok(())
        }
    }

    pub fn create_room(&self, room: &str, description: Option<&str>) -> Result<()> {
        let room_obj = rooms::Room::new(room);
        self.insert_room(&room_obj)?;
        if let Some(desc) = description {
            self.set_room_kv(&room_obj.id, "description", Some(desc))?;
        }
        Ok(())
    }

    /// Fork a room - create new room with copied KV and equipment
    pub fn fork_room(&self, source: &str, new_name: &str) -> Result<()> {
        if let Some(source_room) = self.get_room_by_name(source)? {
            // Create new room
            let new_room = rooms::Room::new(new_name);
            self.insert_room(&new_room)?;

            // Copy all KV pairs from source
            let source_kv = self.get_all_room_kv(&source_room.id)?;
            for (key, value) in source_kv {
                self.set_room_kv(&new_room.id, &key, Some(&value))?;
            }

            // Set parent reference
            self.set_room_kv(&new_room.id, "parent", Some(source))?;

            // Copy room equipment (tools, hooks, commands)
            self.copy_room_equipment(&source_room.id, &new_room.id)?;
        }
        Ok(())
    }

    /// Get room parent (for forked rooms)
    pub fn get_parent(&self, room: &str) -> Result<Option<String>> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            self.get_room_kv(&room_obj.id, "parent")
        } else {
            Ok(None)
        }
    }

    /// Get room tags
    pub fn get_tags(&self, room: &str) -> Result<Vec<String>> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            if let Some(tags_json) = self.get_room_kv(&room_obj.id, "tags")? {
                if let Ok(tags) = serde_json::from_str::<Vec<String>>(&tags_json) {
                    return Ok(tags);
                }
            }
        }
        Ok(vec![])
    }

    /// List all asset bindings in a room
    pub fn list_asset_bindings(&self, room: &str) -> Result<Vec<AssetBinding>> {
        use chrono::{TimeZone, Utc};

        if let Some(room_obj) = self.get_room_by_name(room)? {
            let all_kv = self.get_all_room_kv(&room_obj.id)?;
            let bindings = all_kv
                .iter()
                .filter(|(k, _)| k.starts_with("asset."))
                .filter_map(|(k, v)| {
                    let role = k.strip_prefix("asset.")?;
                    let json = serde_json::from_str::<serde_json::Value>(v).ok()?;
                    let bound_at_ms = json["bound_at"].as_i64().unwrap_or(0);
                    Some(AssetBinding {
                        room: room.to_string(),
                        role: role.to_string(),
                        artifact_id: json["artifact_id"].as_str().unwrap_or("").to_string(),
                        notes: json["notes"].as_str().map(|s| s.to_string()),
                        bound_by: json["bound_by"].as_str().unwrap_or("").to_string(),
                        bound_at: Utc
                            .timestamp_millis_opt(bound_at_ms)
                            .single()
                            .unwrap_or_else(Utc::now),
                    })
                })
                .collect();
            Ok(bindings)
        } else {
            Ok(vec![])
        }
    }

    /// Add a public key for a user (creates agent if not exists)
    pub fn add_pubkey(
        &self,
        handle: &str,
        key: &str,
        _key_type: &str,
        _comment: Option<&str>,
    ) -> Result<()> {
        use agents::{Agent, AgentAuth, AgentKind, AuthKind};

        // Get or create agent
        let agent = match self.get_agent_by_name(handle)? {
            Some(a) => a,
            None => {
                let agent = Agent::new(handle, AgentKind::Human);
                self.insert_agent(&agent)?;
                agent
            }
        };

        // Add the pubkey auth
        let auth = AgentAuth::new(&agent.id, AuthKind::Pubkey, key);
        self.upsert_auth(&auth)?;
        Ok(())
    }

    /// Remove a user by handle
    pub fn remove_user(&self, handle: &str) -> Result<bool> {
        if let Some(agent) = self.get_agent_by_name(handle)? {
            self.delete_agent(&agent.id)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Remove a public key
    pub fn remove_pubkey(&self, key: &str) -> Result<bool> {
        use agents::AuthKind;
        self.delete_auth_by_data(AuthKind::Pubkey, key)
    }

    /// List all users (human agents)
    pub fn list_users(&self) -> Result<Vec<UserInfo>> {
        use agents::AgentKind;
        let agents = self.list_agents(Some(AgentKind::Human))?;

        let mut users = Vec::new();
        for agent in agents {
            let auths = self.list_auth_for_agent(&agent.id)?;
            let key_count = auths
                .iter()
                .filter(|a| a.kind == agents::AuthKind::Pubkey)
                .count();

            users.push(UserInfo {
                handle: agent.name,
                created_at: format_timestamp(agent.created_at),
                last_seen: None, // Not tracked in new model
                key_count,
            });
        }
        Ok(users)
    }

    /// List public keys for a user
    pub fn list_keys_for_user(&self, handle: &str) -> Result<Vec<PubkeyInfo>> {
        use agents::AuthKind;

        if let Some(agent) = self.get_agent_by_name(handle)? {
            let auths = self.list_auth_for_agent(&agent.id)?;
            let keys = auths
                .into_iter()
                .filter(|a| a.kind == AuthKind::Pubkey)
                .map(|a| PubkeyInfo {
                    key: a.auth_data.clone(),
                    pubkey: a.auth_data,
                    key_type: "ssh-ed25519".to_string(), // Assumed, not stored separately
                    comment: None,
                    created_at: format_timestamp(a.created_at),
                })
                .collect();
            Ok(keys)
        } else {
            Ok(vec![])
        }
    }

    /// Get all rooms with their info
    pub fn get_all_rooms(&self) -> Result<Vec<RoomInfo>> {
        let rooms = self.list_rooms()?;
        let mut result = Vec::with_capacity(rooms.len());

        for room in rooms {
            let vibe = self.get_room_kv(&room.id, "vibe")?;
            let description = self.get_room_kv(&room.id, "description")?;
            let created_at = format_timestamp(room.created_at);

            result.push(RoomInfo {
                name: room.name,
                vibe,
                description,
                created_at,
            });
        }

        Ok(result)
    }

    pub fn get_room_navigation(&self, room: &str) -> Result<bool> {
        // Default to allowing navigation
        if let Some(room_obj) = self.get_room_by_name(room)? {
            if let Some(val) = self.get_room_kv(&room_obj.id, "navigation")? {
                return Ok(val != "false");
            }
        }
        Ok(true)
    }

    pub fn set_room_navigation(&self, room: &str, enabled: bool) -> Result<()> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            self.set_room_kv(
                &room_obj.id,
                "navigation",
                Some(if enabled { "true" } else { "false" }),
            )?;
        }
        Ok(())
    }

    // =============================================================================
    // NEW HELPER METHODS FOR MIGRATION
    // =============================================================================

    /// Get or create the main chat buffer for a room (by name)
    ///
    /// Convenience method that takes a room name instead of room_id.
    /// Creates both the room and buffer if they don't exist.
    pub fn get_or_create_room_buffer(&self, room_name: &str) -> Result<buffers::Buffer> {
        // Ensure room exists
        let room = if let Some(r) = self.get_room_by_name(room_name)? {
            r
        } else {
            let r = rooms::Room::new(room_name);
            self.insert_room(&r)?;
            r
        };

        // Get or create the chat buffer
        self.get_or_create_room_chat_buffer(&room.id)
    }

    /// Get the buffer ID for a room (by name), or None if room doesn't exist
    pub fn get_room_buffer_id(&self, room_name: &str) -> Result<Option<String>> {
        if let Some(room) = self.get_room_by_name(room_name)? {
            let buffer = self.get_or_create_room_chat_buffer(&room.id)?;
            Ok(Some(buffer.id))
        } else {
            Ok(None)
        }
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
