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

    /// Add a ledger entry to a room's chat buffer
    pub fn add_ledger_entry(&self, room: &str, entry: &crate::display::LedgerEntry) -> Result<()> {
        use crate::display::{EntryContent, EntrySource};
        use rows::Row;

        // Get room and buffer
        let room_obj = match self.get_room_by_name(room)? {
            Some(r) => r,
            None => return Ok(()), // Room doesn't exist, ignore
        };
        let buffer = self.get_or_create_room_chat_buffer(&room_obj.id)?;

        // Map EntrySource to source_agent_id
        let source_agent_id = match &entry.source {
            EntrySource::User(name) => self.get_agent_by_name(name)?.map(|a| a.id),
            EntrySource::Model { name, .. } => self.get_agent_by_name(name)?.map(|a| a.id),
            EntrySource::System | EntrySource::Command { .. } => None,
        };

        // Map EntryContent to content_method and content
        let (content_method, content) = match &entry.content {
            EntryContent::Chat(text) => {
                let method = match &entry.source {
                    EntrySource::User(_) => "message.user",
                    EntrySource::Model { .. } => "message.model",
                    _ => "message.system",
                };
                (method, Some(text.clone()))
            }
            EntryContent::CommandOutput(text) => ("command.output", Some(text.clone())),
            EntryContent::Status(kind) => {
                use crate::display::StatusKind;
                let method = match kind {
                    StatusKind::Pending => "status.pending",
                    StatusKind::Thinking => "status.thinking",
                    StatusKind::RunningTool(_) => "status.running",
                    StatusKind::Connecting => "status.connecting",
                    StatusKind::Complete => "status.complete",
                };
                (method, None)
            }
            EntryContent::RoomHeader { name, description } => (
                "room.header",
                Some(format!(
                    "{}\n{}",
                    name,
                    description.as_deref().unwrap_or("")
                )),
            ),
            EntryContent::Welcome { username } => ("system.welcome", Some(username.clone())),
            EntryContent::HistorySeparator { label } => ("meta.separator", Some(label.clone())),
            EntryContent::Error(msg) => ("status.error", Some(msg.clone())),
            EntryContent::Presence { user, action } => {
                use crate::display::PresenceAction;
                let method = match action {
                    PresenceAction::Join => "presence.join",
                    PresenceAction::Leave => "presence.leave",
                };
                (method, Some(user.clone()))
            }
            EntryContent::Compaction(summary) => ("meta.compaction", Some(summary.clone())),
        };

        let mut row = Row::new(&buffer.id, content_method);
        row.source_agent_id = source_agent_id;
        row.content = content;
        row.mutable = entry.mutable;
        row.ephemeral = entry.ephemeral;
        row.collapsed = entry.collapsible;

        self.append_row(&mut row)?;
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

    /// Add a journal entry (as Row with tag)
    pub fn add_journal_entry(
        &self,
        room: &str,
        author: &str,
        content: &str,
        kind: crate::world::JournalKind,
    ) -> Result<()> {
        use rows::Row;

        // Get room and buffer
        let room_obj = match self.get_room_by_name(room)? {
            Some(r) => r,
            None => return Ok(()),
        };
        let buffer = self.get_or_create_room_chat_buffer(&room_obj.id)?;

        // Get author's agent ID if they exist
        let source_agent_id = self.get_agent_by_name(author)?.map(|a| a.id);

        // Create row with journal content_method
        let mut row = Row::new(&buffer.id, "note.user");
        row.source_agent_id = source_agent_id;
        row.content = Some(content.to_string());
        self.append_row(&mut row)?;

        // Add tag for the journal kind
        let tag = format!("#{}", kind.as_str());
        self.add_row_tag(&row.id, &tag)?;

        Ok(())
    }

    /// Get journal entries (Rows with journal tags)
    pub fn get_journal_entries(
        &self,
        room: &str,
        kind: Option<crate::world::JournalKind>,
        limit: usize,
    ) -> Result<Vec<JournalEntry>> {
        use crate::world::JournalKind;

        // Get room and buffer
        let room_obj = match self.get_room_by_name(room)? {
            Some(r) => r,
            None => return Ok(vec![]),
        };
        let buffer = self.get_or_create_room_chat_buffer(&room_obj.id)?;

        // Get rows that are journal entries
        let rows = self.list_recent_buffer_rows(&buffer.id, limit * 4)?; // Fetch more to filter

        let mut entries = Vec::new();
        for row in rows {
            // Check if this row has a journal tag
            let tags = self.get_row_tags(&row.id)?;
            let journal_tag = tags.iter().find(|t| {
                t.starts_with("#note")
                    || t.starts_with("#decision")
                    || t.starts_with("#idea")
                    || t.starts_with("#milestone")
                    || t.starts_with("#question")
            });

            if let Some(tag) = journal_tag {
                // Parse the kind from the tag
                let tag_kind = match tag.as_str() {
                    "#note" => JournalKind::Note,
                    "#decision" => JournalKind::Decision,
                    "#idea" => JournalKind::Idea,
                    "#milestone" => JournalKind::Milestone,
                    "#question" => JournalKind::Question,
                    _ => continue,
                };

                // Filter by kind if specified
                if let Some(ref k) = kind {
                    if std::mem::discriminant(&tag_kind) != std::mem::discriminant(k) {
                        continue;
                    }
                }

                use chrono::{TimeZone, Utc};
                let timestamp = Utc
                    .timestamp_millis_opt(row.created_at)
                    .single()
                    .unwrap_or_else(Utc::now);
                entries.push(JournalEntry {
                    id: 0, // Legacy, use row id if needed
                    room: room.to_string(),
                    kind: tag_kind,
                    content: row.content.unwrap_or_default(),
                    author: "unknown".to_string(), // Would need to look up agent
                    created_at: format_timestamp(row.created_at),
                    timestamp,
                });

                if entries.len() >= limit {
                    break;
                }
            }
        }

        Ok(entries)
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

    /// Get room inspirations
    pub fn get_inspirations(&self, room: &str) -> Result<Vec<Inspiration>> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            let all_kv = self.get_all_room_kv(&room_obj.id)?;
            let inspirations = all_kv
                .iter()
                .filter(|(k, _)| k.starts_with("inspiration."))
                .map(|(_, v)| Inspiration { content: v.clone() })
                .collect();
            Ok(inspirations)
        } else {
            Ok(vec![])
        }
    }

    /// Add an inspiration
    pub fn add_inspiration(&self, room: &str, content: &str, _added_by: &str) -> Result<()> {
        if let Some(room_obj) = self.get_room_by_name(room)? {
            // Generate unique key
            let key = format!("inspiration.{}", new_id());
            self.set_room_kv(&room_obj.id, &key, Some(content))?;
        }
        Ok(())
    }

    /// List prompts for a room (stored as scripts with room:name naming)
    pub fn list_prompts(&self, room: &str) -> Result<Vec<RoomPrompt>> {
        use scripts::ScriptKind;

        let prefix = format!("{}:", room);
        let scripts = self.list_scripts(Some(ScriptKind::Handler))?;

        let prompts = scripts
            .into_iter()
            .filter_map(|s| {
                let name = s.name.as_ref()?;
                let short_name = name.strip_prefix(&prefix)?;
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
        use scripts::LuaScript;

        let script_name = format!("{}:{}", room, name);
        let script = LuaScript::new(script_name, "handler", prompt);
        self.insert_script(&script)?;
        Ok(())
    }

    /// Get a prompt from a room
    pub fn get_prompt(&self, room: &str, name: &str) -> Result<Option<RoomPrompt>> {
        let script_name = format!("{}:{}", room, name);
        if let Some(script) = self.get_script_by_name(&script_name)? {
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
        let script_name = format!("{}:{}", room, name);
        if let Some(script) = self.get_script_by_name(&script_name)? {
            self.delete_script(&script.id)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// List all targets that have prompt slots in a room
    pub fn list_targets_with_slots(&self, room: &str) -> Result<Vec<(String, String)>> {
        // Get room to find room_id
        let room_obj = self.get_room_by_name(room)?;
        let room_id = match room_obj {
            Some(r) => r.id,
            None => return Ok(Vec::new()),
        };

        self.list_targets_with_wrap_rules(&room_id)
    }

    /// Get all prompt slots for a target in a room
    pub fn get_target_slots(&self, room: &str, target: &str) -> Result<Vec<PromptSlot>> {
        // Get room to find room_id
        let room_obj = self.get_room_by_name(room)?;
        let room_id = match room_obj {
            Some(r) => r.id,
            None => return Ok(Vec::new()),
        };

        let rules = self.list_wrap_rules_for_target(&room_id, target)?;

        let mut slots = Vec::new();
        for rule in rules {
            // Parse name format: "target_type:prompt_name"
            let (target_type, prompt_name) = match &rule.name {
                Some(name) => {
                    let parts: Vec<&str> = name.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        (parts[0].to_string(), parts[1].to_string())
                    } else {
                        ("unknown".to_string(), name.clone())
                    }
                }
                None => ("unknown".to_string(), "unnamed".to_string()),
            };

            // Get script content
            let content = self.get_script(&rule.script_id)?.map(|s| s.code);

            slots.push(PromptSlot {
                index: rule.priority as usize,
                prompt_name,
                content,
                target_type,
            });
        }

        Ok(slots)
    }

    /// Set (create or update) a prompt in a room
    pub fn set_prompt(
        &self,
        room: &str,
        name: &str,
        content: &str,
        _created_by: &str,
    ) -> Result<()> {
        use scripts::LuaScript;

        let script_name = format!("{}:{}", room, name);

        // Check if script exists
        if let Some(existing) = self.get_script_by_name(&script_name)? {
            // Update existing script
            self.update_script_code(&existing.id, content)?;
        } else {
            // Create new script
            let script = LuaScript::new(&script_name, "handler", content);
            self.insert_script(&script)?;
        }
        Ok(())
    }

    /// Push a prompt slot to the end of a target's slots
    pub fn push_slot(
        &self,
        room: &str,
        target: &str,
        target_type: &str,
        prompt_name: &str,
    ) -> Result<()> {
        use rules::{ActionSlot, RoomRule, TriggerKind};

        // Get room to find room_id
        let room_obj = self
            .get_room_by_name(room)?
            .ok_or_else(|| anyhow::anyhow!("Room not found: {}", room))?;

        // Find the prompt script (format: room:prompt_name)
        let script_name = format!("{}:{}", room, prompt_name);
        let script = self
            .get_script_by_name(&script_name)?
            .ok_or_else(|| anyhow::anyhow!("Prompt not found: {}", prompt_name))?;

        // Get max priority for target
        let max_priority = self.max_wrap_priority(&room_obj.id, target)?;
        let new_priority = max_priority.map(|p| p + 1.0).unwrap_or(0.0);

        // Create new rule
        let mut rule = RoomRule::row_trigger(&room_obj.id, &script.id, ActionSlot::Wrap);
        rule.trigger_kind = TriggerKind::Row; // Wrap rules are row-triggered
        rule.match_source_agent = Some(target.to_string());
        rule.name = Some(format!("{}:{}", target_type, prompt_name));
        rule.priority = new_priority;

        self.insert_rule(&rule)?;

        Ok(())
    }

    /// Remove the last (highest priority) slot from a target
    pub fn pop_slot(&self, room: &str, target: &str) -> Result<bool> {
        // Get room to find room_id
        let room_obj = self.get_room_by_name(room)?;
        let room_id = match room_obj {
            Some(r) => r.id,
            None => return Ok(false),
        };

        // Get max priority
        let max_priority = self.max_wrap_priority(&room_id, target)?;
        match max_priority {
            Some(priority) => self.delete_wrap_rule_by_priority(&room_id, target, priority),
            None => Ok(false),
        }
    }

    /// Remove a slot by index from a target
    pub fn rm_slot(&self, room: &str, target: &str, index: i64) -> Result<bool> {
        // Get room to find room_id
        let room_obj = self.get_room_by_name(room)?;
        let room_id = match room_obj {
            Some(r) => r.id,
            None => return Ok(false),
        };

        // Delete rule by priority (index = priority)
        self.delete_wrap_rule_by_priority(&room_id, target, index as f64)
    }

    /// Insert a slot at a specific index, shifting others up
    pub fn insert_slot(
        &self,
        room: &str,
        target: &str,
        target_type: &str,
        index: i64,
        prompt_name: &str,
    ) -> Result<()> {
        use rules::{ActionSlot, RoomRule, TriggerKind};

        // Get room to find room_id
        let room_obj = self
            .get_room_by_name(room)?
            .ok_or_else(|| anyhow::anyhow!("Room not found: {}", room))?;

        // Find the prompt script (format: room:prompt_name)
        let script_name = format!("{}:{}", room, prompt_name);
        let script = self
            .get_script_by_name(&script_name)?
            .ok_or_else(|| anyhow::anyhow!("Prompt not found: {}", prompt_name))?;

        // Shift existing rules at or above this index up by 1
        self.shift_wrap_priorities(&room_obj.id, target, index as f64)?;

        // Create new rule at the specified index
        let mut rule = RoomRule::row_trigger(&room_obj.id, &script.id, ActionSlot::Wrap);
        rule.trigger_kind = TriggerKind::Row;
        rule.match_source_agent = Some(target.to_string());
        rule.name = Some(format!("{}:{}", target_type, prompt_name));
        rule.priority = index as f64;

        self.insert_rule(&rule)?;

        Ok(())
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

    /// Fork a room - create new room with copied KV
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
        }
        Ok(())
    }

    #[allow(unused_variables)]
    /// Get recent entries from a room's chat buffer (for wrap.lua context)
    pub fn recent_entries(
        &self,
        room: &str,
        limit: usize,
    ) -> Result<Vec<crate::display::LedgerEntry>> {
        use crate::display::{
            EntryContent, EntryId, EntrySource, LedgerEntry, PresenceAction, StatusKind,
        };
        use chrono::{TimeZone, Utc};

        // Get room and buffer
        let room_obj = match self.get_room_by_name(room)? {
            Some(r) => r,
            None => return Ok(vec![]),
        };
        let buffer = self.get_or_create_room_chat_buffer(&room_obj.id)?;

        // Get recent rows
        let rows = self.list_recent_buffer_rows(&buffer.id, limit)?;

        // Convert to LedgerEntry
        let entries = rows
            .into_iter()
            .enumerate()
            .map(|(i, row)| {
                // Parse timestamp
                let timestamp = Utc
                    .timestamp_millis_opt(row.created_at)
                    .single()
                    .unwrap_or_else(Utc::now);

                // Map source
                let source = if let Some(agent_id) = &row.source_agent_id {
                    if let Ok(Some(agent)) = self.get_agent(agent_id) {
                        match agent.kind {
                            agents::AgentKind::Human => EntrySource::User(agent.name),
                            agents::AgentKind::Model => EntrySource::Model {
                                name: agent.name,
                                is_streaming: false,
                            },
                            _ => EntrySource::System,
                        }
                    } else {
                        EntrySource::System
                    }
                } else {
                    EntrySource::System
                };

                // Map content_method back to EntryContent
                let content_text = row.content.clone().unwrap_or_default();
                let content = match row.content_method.as_str() {
                    "message.user" | "message.model" | "message.system" => {
                        EntryContent::Chat(content_text)
                    }
                    "command.output" => EntryContent::CommandOutput(content_text),
                    "status.pending" => EntryContent::Status(StatusKind::Pending),
                    "status.thinking" => EntryContent::Status(StatusKind::Thinking),
                    "status.running" => EntryContent::Status(StatusKind::RunningTool(None)),
                    "status.connecting" => EntryContent::Status(StatusKind::Connecting),
                    "status.complete" => EntryContent::Status(StatusKind::Complete),
                    "status.error" => EntryContent::Error(content_text),
                    "room.header" => {
                        let parts: Vec<&str> = content_text.splitn(2, '\n').collect();
                        EntryContent::RoomHeader {
                            name: parts.first().unwrap_or(&"").to_string(),
                            description: parts.get(1).map(|s| s.to_string()),
                        }
                    }
                    "system.welcome" => EntryContent::Welcome {
                        username: content_text,
                    },
                    "meta.separator" => EntryContent::HistorySeparator {
                        label: content_text,
                    },
                    "presence.join" => EntryContent::Presence {
                        user: content_text,
                        action: PresenceAction::Join,
                    },
                    "presence.leave" => EntryContent::Presence {
                        user: content_text,
                        action: PresenceAction::Leave,
                    },
                    "meta.compaction" => EntryContent::Compaction(content_text),
                    _ => EntryContent::Chat(content_text),
                };

                LedgerEntry {
                    id: EntryId(i as u64),
                    timestamp,
                    source,
                    content,
                    mutable: row.mutable,
                    ephemeral: row.ephemeral,
                    collapsible: row.collapsed,
                }
            })
            .collect();

        Ok(entries)
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
            self.set_room_kv(
                &room_obj.id,
                "navigation",
                Some(if enabled { "true" } else { "false" }),
            )?;
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
