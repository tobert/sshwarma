//! Equipment CRUD operations
//!
//! Equipment represents things active in a context (room or agent) with slots.
//! Slots determine how the thing is used: NULL for general availability,
//! 'command:X' for slash commands, 'hook:wrap' for context composition,
//! 'hook:background' for periodic execution.

use super::things::{Thing, ThingKind};
use super::{new_id, now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// Room equipment - a thing equipped in a room with optional slot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomEquip {
    pub id: String,
    pub room_id: String,
    pub thing_id: String,
    pub slot: Option<String>, // NULL, 'command:fish', 'hook:wrap', 'hook:background'
    pub config: Option<String>, // JSON config
    pub priority: f64,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
}

impl RoomEquip {
    /// Create a new room equipment entry
    pub fn new(room_id: impl Into<String>, thing_id: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            room_id: room_id.into(),
            thing_id: thing_id.into(),
            slot: None,
            config: None,
            priority: 0.0,
            created_at: now_ms(),
            deleted_at: None,
        }
    }

    /// Set slot (builder pattern)
    pub fn with_slot(mut self, slot: impl Into<String>) -> Self {
        self.slot = Some(slot.into());
        self
    }

    /// Set config JSON (builder pattern)
    pub fn with_config(mut self, config: impl Into<String>) -> Self {
        self.config = Some(config.into());
        self
    }

    /// Set priority (builder pattern)
    pub fn with_priority(mut self, priority: f64) -> Self {
        self.priority = priority;
        self
    }
}

/// Agent equipment - a thing equipped by an agent with optional slot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEquip {
    pub id: String,
    pub agent_id: String,
    pub thing_id: String,
    pub slot: Option<String>,
    pub config: Option<String>,
    pub priority: f64,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
}

impl AgentEquip {
    /// Create a new agent equipment entry
    pub fn new(agent_id: impl Into<String>, thing_id: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            agent_id: agent_id.into(),
            thing_id: thing_id.into(),
            slot: None,
            config: None,
            priority: 0.0,
            created_at: now_ms(),
            deleted_at: None,
        }
    }

    /// Set slot (builder pattern)
    pub fn with_slot(mut self, slot: impl Into<String>) -> Self {
        self.slot = Some(slot.into());
        self
    }

    /// Set config JSON (builder pattern)
    pub fn with_config(mut self, config: impl Into<String>) -> Self {
        self.config = Some(config.into());
        self
    }

    /// Set priority (builder pattern)
    pub fn with_priority(mut self, priority: f64) -> Self {
        self.priority = priority;
        self
    }
}

/// Room equipment with joined thing data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomEquippedThing {
    pub equip_id: String,
    pub room_id: String,
    pub slot: Option<String>,
    pub config: Option<String>,
    pub priority: f64,
    pub thing: Thing,
}

/// Agent equipment with joined thing data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEquippedThing {
    pub equip_id: String,
    pub agent_id: String,
    pub slot: Option<String>,
    pub config: Option<String>,
    pub priority: f64,
    pub thing: Thing,
}

// =============================================================================
// Slot filter semantics
// =============================================================================
// - None     -> return ALL equipment (any slot value)
// - Some("") -> return only where slot IS NULL (general availability)
// - Some("hook:wrap") -> return only that specific slot

/// Convert slot filter to SQL condition
fn slot_filter_sql(filter: Option<&str>) -> (&'static str, Option<&str>) {
    match filter {
        None => ("1=1", None),                    // No filter, match all
        Some("") => ("e.slot IS NULL", None),     // Match NULL slots only
        Some(slot) => ("e.slot = ?", Some(slot)), // Match specific slot
    }
}

// =============================================================================
// Room equipment operations
// =============================================================================

impl Database {
    /// Insert room equipment
    pub fn insert_room_equip(&self, equip: &RoomEquip) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO room_equip (id, room_id, thing_id, slot, config, priority, created_at, deleted_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![
                equip.id,
                equip.room_id,
                equip.thing_id,
                equip.slot,
                equip.config,
                equip.priority,
                equip.created_at,
                equip.deleted_at,
            ],
        )
        .context("failed to insert room_equip")?;
        Ok(())
    }

    /// Equip a thing in a room (upsert)
    pub fn room_equip(
        &self,
        room_id: &str,
        thing_id: &str,
        slot: Option<&str>,
        config: Option<&str>,
        priority: f64,
    ) -> Result<String> {
        let conn = self.conn()?;
        let id = new_id();
        let now = now_ms();

        conn.execute(
            r#"INSERT INTO room_equip (id, room_id, thing_id, slot, config, priority, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
               ON CONFLICT(room_id, thing_id, slot) WHERE deleted_at IS NULL
               DO UPDATE SET
                   config = excluded.config,
                   priority = excluded.priority,
                   deleted_at = NULL"#,
            params![id, room_id, thing_id, slot, config, priority, now],
        )
        .context("failed to room_equip")?;
        Ok(id)
    }

    /// Unequip a thing from a room (soft delete)
    pub fn room_unequip(&self, room_id: &str, thing_id: &str, slot: Option<&str>) -> Result<()> {
        let conn = self.conn()?;
        let now = now_ms();

        if let Some(slot_val) = slot {
            conn.execute(
                "UPDATE room_equip SET deleted_at = ?4 WHERE room_id = ?1 AND thing_id = ?2 AND slot = ?3",
                params![room_id, thing_id, slot_val, now],
            )
            .context("failed to room_unequip")?;
        } else {
            conn.execute(
                "UPDATE room_equip SET deleted_at = ?3 WHERE room_id = ?1 AND thing_id = ?2 AND slot IS NULL",
                params![room_id, thing_id, now],
            )
            .context("failed to room_unequip")?;
        }
        Ok(())
    }

    /// Get room equipment with optional slot filter
    /// - None: all equipment
    /// - Some(""): only general availability (slot IS NULL)
    /// - Some("hook:wrap"): only specific slot
    pub fn get_room_equipment(
        &self,
        room_id: &str,
        slot_filter: Option<&str>,
    ) -> Result<Vec<RoomEquippedThing>> {
        let conn = self.conn()?;

        // Build query based on filter
        let (slot_condition, slot_param) = slot_filter_sql(slot_filter);

        let query = format!(
            r#"SELECT e.id, e.room_id, e.slot, e.config, e.priority,
                      t.id, t.parent_id, t.kind, t.name, t.qualified_name, t.description,
                      t.content, t.uri, t.metadata, t.code, t.default_slot, t.params,
                      t.available, t.created_at, t.updated_at, t.deleted_at, t.created_by, t.copied_from
               FROM room_equip e
               JOIN things t ON e.thing_id = t.id
               WHERE e.room_id = ?1
                 AND e.deleted_at IS NULL
                 AND t.deleted_at IS NULL
                 AND {}
               ORDER BY e.priority, t.name"#,
            slot_condition
        );

        let mut stmt = conn.prepare(&query)?;

        let rows = if let Some(slot_val) = slot_param {
            stmt.query_map(params![room_id, slot_val], Self::room_equipped_from_row)?
        } else {
            stmt.query_map(params![room_id], Self::room_equipped_from_row)?
        };

        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get room equipment")
    }

    /// Get room equipment for available tools only (general availability)
    pub fn get_room_equipment_tools(&self, room_id: &str) -> Result<Vec<RoomEquippedThing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT e.id, e.room_id, e.slot, e.config, e.priority,
                      t.id, t.parent_id, t.kind, t.name, t.qualified_name, t.description,
                      t.content, t.uri, t.metadata, t.code, t.default_slot, t.params,
                      t.available, t.created_at, t.updated_at, t.deleted_at, t.created_by, t.copied_from
               FROM room_equip e
               JOIN things t ON e.thing_id = t.id
               WHERE e.room_id = ?1
                 AND e.deleted_at IS NULL
                 AND e.slot IS NULL
                 AND t.deleted_at IS NULL
                 AND t.kind = 'tool'
                 AND t.available = 1
               ORDER BY e.priority, t.name"#,
        )?;

        let rows = stmt.query_map(params![room_id], Self::room_equipped_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get room equipment tools")
    }

    /// Copy room equipment from one room to another
    pub fn copy_room_equipment(&self, from_room_id: &str, to_room_id: &str) -> Result<()> {
        // Get existing equipment and insert copies for new room
        let existing = self.get_room_equipment(from_room_id, None)?;
        for eq in existing {
            self.room_equip(
                to_room_id,
                &eq.thing.id,
                eq.slot.as_deref(),
                eq.config.as_deref(),
                eq.priority,
            )?;
        }

        tracing::debug!(
            from = from_room_id,
            to = to_room_id,
            "copied room equipment"
        );

        Ok(())
    }

    fn room_equipped_from_row(row: &rusqlite::Row) -> rusqlite::Result<RoomEquippedThing> {
        let kind_str: String = row.get(7)?;
        let kind = ThingKind::parse(&kind_str).unwrap_or(ThingKind::Tool);
        Ok(RoomEquippedThing {
            equip_id: row.get(0)?,
            room_id: row.get(1)?,
            slot: row.get(2)?,
            config: row.get(3)?,
            priority: row.get(4)?,
            thing: Thing {
                id: row.get(5)?,
                parent_id: row.get(6)?,
                kind,
                name: row.get(8)?,
                qualified_name: row.get(9)?,
                description: row.get(10)?,
                content: row.get(11)?,
                uri: row.get(12)?,
                metadata: row.get(13)?,
                code: row.get(14)?,
                default_slot: row.get(15)?,
                params: row.get(16)?,
                available: row.get(17)?,
                created_at: row.get(18)?,
                updated_at: row.get(19)?,
                deleted_at: row.get(20)?,
                created_by: row.get(21)?,
                copied_from: row.get(22)?,
            },
        })
    }
}

// =============================================================================
// Agent equipment operations
// =============================================================================

impl Database {
    /// Insert agent equipment
    pub fn insert_agent_equip(&self, equip: &AgentEquip) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO agent_equip (id, agent_id, thing_id, slot, config, priority, created_at, deleted_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![
                equip.id,
                equip.agent_id,
                equip.thing_id,
                equip.slot,
                equip.config,
                equip.priority,
                equip.created_at,
                equip.deleted_at,
            ],
        )
        .context("failed to insert agent_equip")?;
        Ok(())
    }

    /// Equip a thing for an agent (upsert)
    pub fn agent_equip(
        &self,
        agent_id: &str,
        thing_id: &str,
        slot: Option<&str>,
        config: Option<&str>,
        priority: f64,
    ) -> Result<String> {
        let conn = self.conn()?;
        let id = new_id();
        let now = now_ms();

        conn.execute(
            r#"INSERT INTO agent_equip (id, agent_id, thing_id, slot, config, priority, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
               ON CONFLICT(agent_id, thing_id, slot) WHERE deleted_at IS NULL
               DO UPDATE SET
                   config = excluded.config,
                   priority = excluded.priority,
                   deleted_at = NULL"#,
            params![id, agent_id, thing_id, slot, config, priority, now],
        )
        .context("failed to agent_equip")?;
        Ok(id)
    }

    /// Unequip a thing from an agent (soft delete)
    pub fn agent_unequip(&self, agent_id: &str, thing_id: &str, slot: Option<&str>) -> Result<()> {
        let conn = self.conn()?;
        let now = now_ms();

        if let Some(slot_val) = slot {
            conn.execute(
                "UPDATE agent_equip SET deleted_at = ?4 WHERE agent_id = ?1 AND thing_id = ?2 AND slot = ?3",
                params![agent_id, thing_id, slot_val, now],
            )
            .context("failed to agent_unequip")?;
        } else {
            conn.execute(
                "UPDATE agent_equip SET deleted_at = ?3 WHERE agent_id = ?1 AND thing_id = ?2 AND slot IS NULL",
                params![agent_id, thing_id, now],
            )
            .context("failed to agent_unequip")?;
        }
        Ok(())
    }

    /// Get agent equipment with optional slot filter
    pub fn get_agent_equipment(
        &self,
        agent_id: &str,
        slot_filter: Option<&str>,
    ) -> Result<Vec<AgentEquippedThing>> {
        let conn = self.conn()?;

        let (slot_condition, slot_param) = slot_filter_sql(slot_filter);

        let query = format!(
            r#"SELECT e.id, e.agent_id, e.slot, e.config, e.priority,
                      t.id, t.parent_id, t.kind, t.name, t.qualified_name, t.description,
                      t.content, t.uri, t.metadata, t.code, t.default_slot, t.params,
                      t.available, t.created_at, t.updated_at, t.deleted_at, t.created_by, t.copied_from
               FROM agent_equip e
               JOIN things t ON e.thing_id = t.id
               WHERE e.agent_id = ?1
                 AND e.deleted_at IS NULL
                 AND t.deleted_at IS NULL
                 AND {}
               ORDER BY e.priority, t.name"#,
            slot_condition
        );

        let mut stmt = conn.prepare(&query)?;

        let rows = if let Some(slot_val) = slot_param {
            stmt.query_map(params![agent_id, slot_val], Self::agent_equipped_from_row)?
        } else {
            stmt.query_map(params![agent_id], Self::agent_equipped_from_row)?
        };

        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get agent equipment")
    }

    fn agent_equipped_from_row(row: &rusqlite::Row) -> rusqlite::Result<AgentEquippedThing> {
        let kind_str: String = row.get(7)?;
        let kind = ThingKind::parse(&kind_str).unwrap_or(ThingKind::Tool);
        Ok(AgentEquippedThing {
            equip_id: row.get(0)?,
            agent_id: row.get(1)?,
            slot: row.get(2)?,
            config: row.get(3)?,
            priority: row.get(4)?,
            thing: Thing {
                id: row.get(5)?,
                parent_id: row.get(6)?,
                kind,
                name: row.get(8)?,
                qualified_name: row.get(9)?,
                description: row.get(10)?,
                content: row.get(11)?,
                uri: row.get(12)?,
                metadata: row.get(13)?,
                code: row.get(14)?,
                default_slot: row.get(15)?,
                params: row.get(16)?,
                available: row.get(17)?,
                created_at: row.get(18)?,
                updated_at: row.get(19)?,
                deleted_at: row.get(20)?,
                created_by: row.get(21)?,
                copied_from: row.get(22)?,
            },
        })
    }
}

// =============================================================================
// Merged equipment (for LLM calls)
// =============================================================================

impl Database {
    /// Get merged equipment from room + agent for LLM calls
    /// Room equipment takes priority over agent equipment
    pub fn get_merged_equipment(
        &self,
        room_id: &str,
        agent_id: &str,
        slot_filter: Option<&str>,
    ) -> Result<Vec<RoomEquippedThing>> {
        // Get room equipment first
        let mut equipment = self.get_room_equipment(room_id, slot_filter)?;

        // Get agent equipment
        let agent_equipment = self.get_agent_equipment(agent_id, slot_filter)?;

        // Track thing IDs we already have (room wins)
        let existing_things: std::collections::HashSet<_> =
            equipment.iter().map(|e| e.thing.id.clone()).collect();

        // Add agent equipment for things not already equipped in room
        for ae in agent_equipment {
            if !existing_things.contains(&ae.thing.id) {
                equipment.push(RoomEquippedThing {
                    equip_id: ae.equip_id,
                    room_id: room_id.to_string(), // Normalize to room context
                    slot: ae.slot,
                    config: ae.config,
                    priority: ae.priority + 1000.0, // Agent equipment comes after room
                    thing: ae.thing,
                });
            }
        }

        // Re-sort by priority
        equipment.sort_by(|a, b| a.priority.partial_cmp(&b.priority).unwrap());

        Ok(equipment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_room_equip_unequip() -> Result<()> {
        let db = Database::in_memory()?;

        // Create actual room (in rooms table)
        use crate::db::rooms::Room;
        let room = Room::new("workshop");
        db.insert_room(&room)?;

        // Create tool thing
        let tool = Thing::tool("look", "sshwarma:look");
        db.insert_thing(&tool)?;

        // Equip with slot
        db.room_equip(&room.id, &tool.id, Some("command:look"), None, 0.0)?;

        // Get equipment for specific slot
        let equipped = db.get_room_equipment(&room.id, Some("command:look"))?;
        assert_eq!(equipped.len(), 1);
        assert_eq!(equipped[0].thing.name, "look");
        assert_eq!(equipped[0].slot, Some("command:look".to_string()));

        // Get all equipment
        let all_equipped = db.get_room_equipment(&room.id, None)?;
        assert_eq!(all_equipped.len(), 1);

        // Get only general availability (none exist)
        let general = db.get_room_equipment(&room.id, Some(""))?;
        assert_eq!(general.len(), 0);

        // Unequip
        db.room_unequip(&room.id, &tool.id, Some("command:look"))?;
        let equipped = db.get_room_equipment(&room.id, None)?;
        assert_eq!(equipped.len(), 0);

        Ok(())
    }

    #[test]
    fn test_agent_equip_unequip() -> Result<()> {
        let db = Database::in_memory()?;

        // Create agent
        use crate::db::agents::{Agent, AgentKind};
        let agent = Agent::new("testuser", AgentKind::Human);
        db.insert_agent(&agent)?;

        let tool = Thing::tool("fish", "atobey:fish");
        db.insert_thing(&tool)?;

        // Equip
        db.agent_equip(&agent.id, &tool.id, Some("command:fish"), None, 0.0)?;

        let equipped = db.get_agent_equipment(&agent.id, Some("command:fish"))?;
        assert_eq!(equipped.len(), 1);
        assert_eq!(equipped[0].thing.name, "fish");

        // Unequip
        db.agent_unequip(&agent.id, &tool.id, Some("command:fish"))?;
        let equipped = db.get_agent_equipment(&agent.id, None)?;
        assert_eq!(equipped.len(), 0);

        Ok(())
    }

    #[test]
    fn test_copy_room_equipment() -> Result<()> {
        let db = Database::in_memory()?;

        use crate::db::rooms::Room;
        let room1 = Room::new("workshop");
        let room2 = Room::new("studio");
        db.insert_room(&room1)?;
        db.insert_room(&room2)?;

        let tool1 = Thing::tool("look", "sshwarma:look");
        let tool2 = Thing::tool("say", "sshwarma:say");
        db.insert_thing(&tool1)?;
        db.insert_thing(&tool2)?;

        db.room_equip(&room1.id, &tool1.id, None, None, 0.0)?;
        db.room_equip(&room1.id, &tool2.id, Some("command:say"), None, 1.0)?;

        db.copy_room_equipment(&room1.id, &room2.id)?;

        let equipped = db.get_room_equipment(&room2.id, None)?;
        assert_eq!(equipped.len(), 2);

        Ok(())
    }

    #[test]
    fn test_merged_equipment() -> Result<()> {
        let db = Database::in_memory()?;

        // Create room and agent
        use crate::db::agents::{Agent, AgentKind};
        use crate::db::rooms::Room;

        let room = Room::new("workshop");
        db.insert_room(&room)?;
        let agent = Agent::new("testuser", AgentKind::Human);
        db.insert_agent(&agent)?;

        // Create tools
        let look = Thing::tool("look", "sshwarma:look");
        let say = Thing::tool("say", "sshwarma:say");
        let fish = Thing::tool("fish", "atobey:fish");
        db.insert_thing(&look)?;
        db.insert_thing(&say)?;
        db.insert_thing(&fish)?;

        // Equip look and say to room (general availability)
        db.room_equip(&room.id, &look.id, None, None, 0.0)?;
        db.room_equip(&room.id, &say.id, None, None, 1.0)?;

        // Equip fish to agent (personal tool)
        db.agent_equip(&agent.id, &fish.id, None, None, 0.0)?;

        // Get merged equipment (should include all 3)
        let merged = db.get_merged_equipment(&room.id, &agent.id, None)?;
        assert_eq!(merged.len(), 3);

        // Room equipment comes first (priority 0.0, 1.0)
        // Agent equipment comes after (priority + 1000.0)
        assert!(merged[0].thing.name == "look" || merged[0].thing.name == "say");
        assert_eq!(merged[2].thing.name, "fish");

        // If agent has same tool as room, room wins
        db.agent_equip(&agent.id, &look.id, None, None, 0.0)?;
        let merged = db.get_merged_equipment(&room.id, &agent.id, None)?;
        assert_eq!(merged.len(), 3); // Still 3, not 4 (room look wins)

        Ok(())
    }

    #[test]
    fn test_slot_filter_semantics() -> Result<()> {
        let db = Database::in_memory()?;

        use crate::db::rooms::Room;
        let room = Room::new("workshop");
        db.insert_room(&room)?;

        // Create tools
        let look = Thing::tool("look", "sshwarma:look");
        let fish = Thing::tool("fish", "atobey:fish");
        let ticker = Thing::tool("ticker", "atobey:ticker");
        let wrapper = Thing::tool("wrapper", "atobey:wrapper");
        db.insert_thing(&look)?;
        db.insert_thing(&fish)?;
        db.insert_thing(&ticker)?;
        db.insert_thing(&wrapper)?;

        // Equip with different slots
        db.room_equip(&room.id, &look.id, None, None, 0.0)?; // General availability (NULL slot)
        db.room_equip(&room.id, &fish.id, Some("command:fish"), None, 0.0)?;
        db.room_equip(
            &room.id,
            &ticker.id,
            Some("hook:background:ui"),
            Some(r#"{"interval_ms":1000}"#),
            0.0,
        )?;
        db.room_equip(&room.id, &wrapper.id, Some("hook:wrap"), None, 0.0)?;

        // None filter: get all
        let all = db.get_room_equipment(&room.id, None)?;
        assert_eq!(all.len(), 4);

        // Empty string filter: get only NULL slot (general availability)
        let general = db.get_room_equipment(&room.id, Some(""))?;
        assert_eq!(general.len(), 1);
        assert_eq!(general[0].thing.name, "look");
        assert!(general[0].slot.is_none());

        // Specific slot filter: command:fish
        let commands = db.get_room_equipment(&room.id, Some("command:fish"))?;
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].thing.name, "fish");

        // Specific slot filter: hook:wrap
        let wrap_hooks = db.get_room_equipment(&room.id, Some("hook:wrap"))?;
        assert_eq!(wrap_hooks.len(), 1);
        assert_eq!(wrap_hooks[0].thing.name, "wrapper");

        // Specific slot filter: hook:background:ui (with config)
        let bg_hooks = db.get_room_equipment(&room.id, Some("hook:background:ui"))?;
        assert_eq!(bg_hooks.len(), 1);
        assert_eq!(bg_hooks[0].thing.name, "ticker");
        assert!(bg_hooks[0].config.as_ref().unwrap().contains("interval_ms"));

        Ok(())
    }
}
