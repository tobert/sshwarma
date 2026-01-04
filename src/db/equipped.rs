//! Equipped CRUD operations
//!
//! Equipped represents what's active in a context (room or agent).
//! This is a many-to-many relationship: multiple rooms can equip the same tool.

use super::{now_ms, Database};
use super::things::{Thing, ThingKind};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// An equipped relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Equipped {
    pub context_id: String,    // room or agent thing ID
    pub thing_id: String,      // tool or data thing ID
    pub priority: f64,         // ordering (lower = first)
    pub created_at: i64,
    pub deleted_at: Option<i64>,
}

impl Equipped {
    /// Create a new equipped relationship
    pub fn new(context_id: impl Into<String>, thing_id: impl Into<String>) -> Self {
        Self {
            context_id: context_id.into(),
            thing_id: thing_id.into(),
            priority: 0.0,
            created_at: now_ms(),
            deleted_at: None,
        }
    }

    /// Set priority (builder pattern)
    pub fn with_priority(mut self, priority: f64) -> Self {
        self.priority = priority;
        self
    }
}

/// Equipped thing with full thing data joined
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquippedThing {
    pub context_id: String,
    pub priority: f64,
    pub thing: Thing,
}

// =============================================================================
// Database operations
// =============================================================================

impl Database {
    /// Equip a thing in a context (creates equipped row)
    pub fn equip(&self, context_id: &str, thing_id: &str, priority: f64) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO equipped (context_id, thing_id, priority, created_at)
               VALUES (?1, ?2, ?3, ?4)
               ON CONFLICT(context_id, thing_id) DO UPDATE SET
                   priority = excluded.priority,
                   deleted_at = NULL"#,
            params![context_id, thing_id, priority, now_ms()],
        )
        .context("failed to equip thing")?;
        Ok(())
    }

    /// Unequip a thing from a context (soft-delete equipped row)
    pub fn unequip(&self, context_id: &str, thing_id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE equipped SET deleted_at = ?3 WHERE context_id = ?1 AND thing_id = ?2",
            params![context_id, thing_id, now_ms()],
        )
        .context("failed to unequip thing")?;
        Ok(())
    }

    /// Check if a thing is equipped in a context
    pub fn is_equipped(&self, context_id: &str, thing_id: &str) -> Result<bool> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT 1 FROM equipped WHERE context_id = ?1 AND thing_id = ?2 AND deleted_at IS NULL",
        )?;
        Ok(stmt.exists(params![context_id, thing_id])?)
    }

    /// Get all equipped things for a context (with full thing data, sorted by priority)
    pub fn get_equipped(&self, context_id: &str) -> Result<Vec<EquippedThing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT e.context_id, e.priority,
                      t.id, t.parent_id, t.kind, t.name, t.qualified_name, t.description,
                      t.content, t.uri, t.metadata, t.available, t.created_at, t.updated_at, t.deleted_at
               FROM equipped e
               JOIN things t ON e.thing_id = t.id
               WHERE e.context_id = ?1
                 AND e.deleted_at IS NULL
                 AND t.deleted_at IS NULL
               ORDER BY e.priority, t.name"#,
        )?;
        let rows = stmt.query_map(params![context_id], |row| {
            let kind_str: String = row.get(4)?;
            let kind = ThingKind::parse(&kind_str).unwrap_or(ThingKind::Data);
            Ok(EquippedThing {
                context_id: row.get(0)?,
                priority: row.get(1)?,
                thing: Thing {
                    id: row.get(2)?,
                    parent_id: row.get(3)?,
                    kind,
                    name: row.get(5)?,
                    qualified_name: row.get(6)?,
                    description: row.get(7)?,
                    content: row.get(8)?,
                    uri: row.get(9)?,
                    metadata: row.get(10)?,
                    available: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                    deleted_at: row.get(14)?,
                },
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get equipped")
    }

    /// Get equipped tools for a context (only tools, only available)
    pub fn get_equipped_tools(&self, context_id: &str) -> Result<Vec<EquippedThing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT e.context_id, e.priority,
                      t.id, t.parent_id, t.kind, t.name, t.qualified_name, t.description,
                      t.content, t.uri, t.metadata, t.available, t.created_at, t.updated_at, t.deleted_at
               FROM equipped e
               JOIN things t ON e.thing_id = t.id
               WHERE e.context_id = ?1
                 AND e.deleted_at IS NULL
                 AND t.deleted_at IS NULL
                 AND t.kind = 'tool'
                 AND t.available = 1
               ORDER BY e.priority, t.name"#,
        )?;
        let rows = stmt.query_map(params![context_id], |row| {
            Ok(EquippedThing {
                context_id: row.get(0)?,
                priority: row.get(1)?,
                thing: Thing {
                    id: row.get(2)?,
                    parent_id: row.get(3)?,
                    kind: ThingKind::Tool,
                    name: row.get(5)?,
                    qualified_name: row.get(6)?,
                    description: row.get(7)?,
                    content: row.get(8)?,
                    uri: row.get(9)?,
                    metadata: row.get(10)?,
                    available: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                    deleted_at: row.get(14)?,
                },
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get equipped tools")
    }

    /// Get equipped data for a context (only data kind)
    pub fn get_equipped_data(&self, context_id: &str) -> Result<Vec<EquippedThing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT e.context_id, e.priority,
                      t.id, t.parent_id, t.kind, t.name, t.qualified_name, t.description,
                      t.content, t.uri, t.metadata, t.available, t.created_at, t.updated_at, t.deleted_at
               FROM equipped e
               JOIN things t ON e.thing_id = t.id
               WHERE e.context_id = ?1
                 AND e.deleted_at IS NULL
                 AND t.deleted_at IS NULL
                 AND t.kind = 'data'
               ORDER BY e.priority, t.name"#,
        )?;
        let rows = stmt.query_map(params![context_id], |row| {
            Ok(EquippedThing {
                context_id: row.get(0)?,
                priority: row.get(1)?,
                thing: Thing {
                    id: row.get(2)?,
                    parent_id: row.get(3)?,
                    kind: ThingKind::Data,
                    name: row.get(5)?,
                    qualified_name: row.get(6)?,
                    description: row.get(7)?,
                    content: row.get(8)?,
                    uri: row.get(9)?,
                    metadata: row.get(10)?,
                    available: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                    deleted_at: row.get(14)?,
                },
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get equipped data")
    }

    /// Get merged equipped tools for room + agent (room wins conflicts)
    /// Returns tools from both contexts, with room-equipped tools taking priority
    pub fn get_merged_equipped_tools(
        &self,
        room_id: &str,
        agent_id: &str,
    ) -> Result<Vec<EquippedThing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT DISTINCT e.context_id, e.priority,
                      t.id, t.parent_id, t.kind, t.name, t.qualified_name, t.description,
                      t.content, t.uri, t.metadata, t.available, t.created_at, t.updated_at, t.deleted_at
               FROM equipped e
               JOIN things t ON e.thing_id = t.id
               WHERE e.context_id IN (?1, ?2)
                 AND e.deleted_at IS NULL
                 AND t.deleted_at IS NULL
                 AND t.kind = 'tool'
                 AND t.available = 1
               ORDER BY
                   CASE WHEN e.context_id = ?1 THEN 0 ELSE 1 END,  -- room first
                   e.priority,
                   t.name"#,
        )?;
        let rows = stmt.query_map(params![room_id, agent_id], |row| {
            Ok(EquippedThing {
                context_id: row.get(0)?,
                priority: row.get(1)?,
                thing: Thing {
                    id: row.get(2)?,
                    parent_id: row.get(3)?,
                    kind: ThingKind::Tool,
                    name: row.get(5)?,
                    qualified_name: row.get(6)?,
                    description: row.get(7)?,
                    content: row.get(8)?,
                    uri: row.get(9)?,
                    metadata: row.get(10)?,
                    available: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                    deleted_at: row.get(14)?,
                },
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get merged equipped tools")
    }

    /// Get merged equipped data for room + agent (room wins conflicts)
    pub fn get_merged_equipped_data(
        &self,
        room_id: &str,
        agent_id: &str,
    ) -> Result<Vec<EquippedThing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT DISTINCT e.context_id, e.priority,
                      t.id, t.parent_id, t.kind, t.name, t.qualified_name, t.description,
                      t.content, t.uri, t.metadata, t.available, t.created_at, t.updated_at, t.deleted_at
               FROM equipped e
               JOIN things t ON e.thing_id = t.id
               WHERE e.context_id IN (?1, ?2)
                 AND e.deleted_at IS NULL
                 AND t.deleted_at IS NULL
                 AND t.kind = 'data'
               ORDER BY
                   CASE WHEN e.context_id = ?1 THEN 0 ELSE 1 END,  -- room first
                   e.priority,
                   t.name"#,
        )?;
        let rows = stmt.query_map(params![room_id, agent_id], |row| {
            Ok(EquippedThing {
                context_id: row.get(0)?,
                priority: row.get(1)?,
                thing: Thing {
                    id: row.get(2)?,
                    parent_id: row.get(3)?,
                    kind: ThingKind::Data,
                    name: row.get(5)?,
                    qualified_name: row.get(6)?,
                    description: row.get(7)?,
                    content: row.get(8)?,
                    uri: row.get(9)?,
                    metadata: row.get(10)?,
                    available: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                    deleted_at: row.get(14)?,
                },
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get merged equipped data")
    }

    /// Copy equipped relationships from one context to another
    /// Used when creating a new room from defaults
    pub fn copy_equipped(&self, from_context: &str, to_context: &str) -> Result<()> {
        let conn = self.conn()?;
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO equipped (context_id, thing_id, priority, created_at)
               SELECT ?2, thing_id, priority, ?3
               FROM equipped
               WHERE context_id = ?1 AND deleted_at IS NULL"#,
            params![from_context, to_context, now],
        )
        .context("failed to copy equipped")?;
        Ok(())
    }

    /// Get max priority in a context (for appending)
    pub fn max_equipped_priority(&self, context_id: &str) -> Result<Option<f64>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT MAX(priority) FROM equipped WHERE context_id = ?1 AND deleted_at IS NULL",
        )?;
        stmt.query_row(params![context_id], |row| row.get(0))
            .optional()
            .context("failed to get max priority")?
            .ok_or_else(|| anyhow::anyhow!("no rows"))
            .or(Ok(None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equip_unequip() -> Result<()> {
        let db = Database::in_memory()?;

        // Create room and tool things
        let room = Thing::room("workshop");
        db.insert_thing(&room)?;

        let tool = Thing::tool("look", "sshwarma:look");
        db.insert_thing(&tool)?;

        // Equip
        db.equip(&room.id, &tool.id, 0.0)?;
        assert!(db.is_equipped(&room.id, &tool.id)?);

        // Get equipped
        let equipped = db.get_equipped(&room.id)?;
        assert_eq!(equipped.len(), 1);
        assert_eq!(equipped[0].thing.name, "look");

        // Unequip
        db.unequip(&room.id, &tool.id)?;
        assert!(!db.is_equipped(&room.id, &tool.id)?);

        let equipped = db.get_equipped(&room.id)?;
        assert_eq!(equipped.len(), 0);

        Ok(())
    }

    #[test]
    fn test_equipped_tools_filter() -> Result<()> {
        let db = Database::in_memory()?;

        let room = Thing::room("workshop");
        db.insert_thing(&room)?;

        // Create tool and data
        let tool = Thing::tool("look", "sshwarma:look");
        db.insert_thing(&tool)?;

        let data = Thing::data("style", "Be concise");
        db.insert_thing(&data)?;

        db.equip(&room.id, &tool.id, 0.0)?;
        db.equip(&room.id, &data.id, 0.0)?;

        // get_equipped_tools should only return tools
        let tools = db.get_equipped_tools(&room.id)?;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].thing.kind, ThingKind::Tool);

        // get_equipped_data should only return data
        let data_items = db.get_equipped_data(&room.id)?;
        assert_eq!(data_items.len(), 1);
        assert_eq!(data_items[0].thing.kind, ThingKind::Data);

        Ok(())
    }

    #[test]
    fn test_copy_equipped() -> Result<()> {
        let db = Database::in_memory()?;

        // Create defaults and tools
        let defaults = Thing::container("defaults");
        db.insert_thing(&defaults)?;

        let tool1 = Thing::tool("look", "sshwarma:look");
        let tool2 = Thing::tool("say", "sshwarma:say");
        db.insert_thing(&tool1)?;
        db.insert_thing(&tool2)?;

        db.equip(&defaults.id, &tool1.id, 0.0)?;
        db.equip(&defaults.id, &tool2.id, 1.0)?;

        // Create new room and copy equipped
        let room = Thing::room("workshop");
        db.insert_thing(&room)?;

        db.copy_equipped(&defaults.id, &room.id)?;

        // Room should have same equipped
        let equipped = db.get_equipped(&room.id)?;
        assert_eq!(equipped.len(), 2);

        Ok(())
    }
}
