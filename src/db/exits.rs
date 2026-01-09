//! Exits CRUD operations
//!
//! Exits connect rooms together for navigation.
//! Each exit is a directed edge: from_thing_id → to_thing_id via direction.

use super::things::Thing;
use super::{now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// An exit between rooms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exit {
    pub from_thing_id: String,
    pub direction: String,
    pub to_thing_id: String,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
}

impl Exit {
    /// Create a new exit
    pub fn new(
        from_thing_id: impl Into<String>,
        direction: impl Into<String>,
        to_thing_id: impl Into<String>,
    ) -> Self {
        Self {
            from_thing_id: from_thing_id.into(),
            direction: direction.into(),
            to_thing_id: to_thing_id.into(),
            created_at: now_ms(),
            deleted_at: None,
        }
    }
}

/// Exit with target room data joined
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitWithTarget {
    pub direction: String,
    pub target: Thing,
}

// =============================================================================
// Database operations
// =============================================================================

impl Database {
    /// Create an exit between rooms
    pub fn create_exit(
        &self,
        from_thing_id: &str,
        direction: &str,
        to_thing_id: &str,
    ) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO exits (from_thing_id, direction, to_thing_id, created_at)
               VALUES (?1, ?2, ?3, ?4)
               ON CONFLICT(from_thing_id, direction) DO UPDATE SET
                   to_thing_id = excluded.to_thing_id,
                   deleted_at = NULL"#,
            params![from_thing_id, direction, to_thing_id, now_ms()],
        )
        .context("failed to create exit")?;
        Ok(())
    }

    /// Create a bidirectional exit (north/south, east/west, etc.)
    pub fn create_bidirectional_exit(
        &self,
        room1_id: &str,
        dir1: &str,
        room2_id: &str,
        dir2: &str,
    ) -> Result<()> {
        self.create_exit(room1_id, dir1, room2_id)?;
        self.create_exit(room2_id, dir2, room1_id)?;
        Ok(())
    }

    /// Delete an exit (soft-delete)
    pub fn delete_exit(&self, from_thing_id: &str, direction: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE exits SET deleted_at = ?3 WHERE from_thing_id = ?1 AND direction = ?2",
            params![from_thing_id, direction, now_ms()],
        )
        .context("failed to delete exit")?;
        Ok(())
    }

    /// Get all exits from a room (with target room data)
    pub fn get_exits_from(&self, from_thing_id: &str) -> Result<Vec<ExitWithTarget>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT e.direction,
                      t.id, t.parent_id, t.kind, t.name, t.qualified_name, t.description,
                      t.content, t.uri, t.metadata, t.code, t.default_slot, t.params,
                      t.available, t.created_at, t.updated_at, t.deleted_at, t.created_by
               FROM exits e
               JOIN things t ON e.to_thing_id = t.id
               WHERE e.from_thing_id = ?1
                 AND e.deleted_at IS NULL
                 AND t.deleted_at IS NULL
               ORDER BY e.direction"#,
        )?;
        let rows = stmt.query_map(params![from_thing_id], |row| {
            use super::things::ThingKind;
            let kind_str: String = row.get(3)?;
            let kind = ThingKind::parse(&kind_str).unwrap_or(ThingKind::Room);
            Ok(ExitWithTarget {
                direction: row.get(0)?,
                target: Thing {
                    id: row.get(1)?,
                    parent_id: row.get(2)?,
                    kind,
                    name: row.get(4)?,
                    qualified_name: row.get(5)?,
                    description: row.get(6)?,
                    content: row.get(7)?,
                    uri: row.get(8)?,
                    metadata: row.get(9)?,
                    code: row.get(10)?,
                    default_slot: row.get(11)?,
                    params: row.get(12)?,
                    available: row.get(13)?,
                    created_at: row.get(14)?,
                    updated_at: row.get(15)?,
                    deleted_at: row.get(16)?,
                    created_by: row.get(17)?,
                },
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get exits")
    }

    /// Get exit by direction
    pub fn get_exit(&self, from_thing_id: &str, direction: &str) -> Result<Option<Exit>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT from_thing_id, direction, to_thing_id, created_at, deleted_at
               FROM exits
               WHERE from_thing_id = ?1 AND direction = ?2 AND deleted_at IS NULL"#,
        )?;
        stmt.query_row(params![from_thing_id, direction], |row| {
            Ok(Exit {
                from_thing_id: row.get(0)?,
                direction: row.get(1)?,
                to_thing_id: row.get(2)?,
                created_at: row.get(3)?,
                deleted_at: row.get(4)?,
            })
        })
        .optional()
        .context("failed to get exit")
    }

    /// Get exits as a simple direction → room_name map (for compatibility)
    pub fn get_exits_map(
        &self,
        from_thing_id: &str,
    ) -> Result<std::collections::HashMap<String, String>> {
        let exits = self.get_exits_from(from_thing_id)?;
        Ok(exits
            .into_iter()
            .map(|e| (e.direction, e.target.name))
            .collect())
    }

    /// Check if there's a path from A to B (simple cycle detection)
    /// Returns true if there's any path, used to prevent cycles
    pub fn has_path(&self, from_id: &str, to_id: &str) -> Result<bool> {
        let conn = self.conn()?;
        // Simple BFS to find if there's a path
        let mut visited = std::collections::HashSet::new();
        let mut queue = vec![from_id.to_string()];

        while let Some(current) = queue.pop() {
            if current == to_id {
                return Ok(true);
            }
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current.clone());

            // Get all exits from current
            let mut stmt = conn.prepare(
                "SELECT to_thing_id FROM exits WHERE from_thing_id = ?1 AND deleted_at IS NULL",
            )?;
            let neighbors: Vec<String> = stmt
                .query_map(params![current], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;

            for neighbor in neighbors {
                if !visited.contains(&neighbor) {
                    queue.push(neighbor);
                }
            }
        }

        Ok(false)
    }
}

/// Common direction pairs for bidirectional exits
pub fn opposite_direction(dir: &str) -> Option<&'static str> {
    match dir.to_lowercase().as_str() {
        "north" | "n" => Some("south"),
        "south" | "s" => Some("north"),
        "east" | "e" => Some("west"),
        "west" | "w" => Some("east"),
        "up" | "u" => Some("down"),
        "down" | "d" => Some("up"),
        "in" => Some("out"),
        "out" => Some("in"),
        _ => None,
    }
}

/// Normalize direction to standard form
pub fn normalize_direction(dir: &str) -> String {
    match dir.to_lowercase().as_str() {
        "n" => "north".to_string(),
        "s" => "south".to_string(),
        "e" => "east".to_string(),
        "w" => "west".to_string(),
        "u" => "up".to_string(),
        "d" => "down".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::things::Thing;
    use super::*;

    #[test]
    fn test_exit_crud() -> Result<()> {
        let db = Database::in_memory()?;

        // Create rooms
        let lobby = Thing::room("lobby");
        let workshop = Thing::room("workshop");
        db.insert_thing(&lobby)?;
        db.insert_thing(&workshop)?;

        // Create exit
        db.create_exit(&lobby.id, "north", &workshop.id)?;

        // Get exits
        let exits = db.get_exits_from(&lobby.id)?;
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0].direction, "north");
        assert_eq!(exits[0].target.name, "workshop");

        // Get as map
        let map = db.get_exits_map(&lobby.id)?;
        assert_eq!(map.get("north"), Some(&"workshop".to_string()));

        // Delete exit
        db.delete_exit(&lobby.id, "north")?;
        let exits = db.get_exits_from(&lobby.id)?;
        assert_eq!(exits.len(), 0);

        Ok(())
    }

    #[test]
    fn test_bidirectional_exit() -> Result<()> {
        let db = Database::in_memory()?;

        let lobby = Thing::room("lobby");
        let workshop = Thing::room("workshop");
        db.insert_thing(&lobby)?;
        db.insert_thing(&workshop)?;

        // Create bidirectional exit
        db.create_bidirectional_exit(&lobby.id, "north", &workshop.id, "south")?;

        // Both directions should work
        let lobby_exits = db.get_exits_from(&lobby.id)?;
        assert_eq!(lobby_exits.len(), 1);
        assert_eq!(lobby_exits[0].direction, "north");

        let workshop_exits = db.get_exits_from(&workshop.id)?;
        assert_eq!(workshop_exits.len(), 1);
        assert_eq!(workshop_exits[0].direction, "south");

        Ok(())
    }

    #[test]
    fn test_has_path() -> Result<()> {
        let db = Database::in_memory()?;

        let a = Thing::room("a");
        let b = Thing::room("b");
        let c = Thing::room("c");
        db.insert_thing(&a)?;
        db.insert_thing(&b)?;
        db.insert_thing(&c)?;

        // a → b → c
        db.create_exit(&a.id, "east", &b.id)?;
        db.create_exit(&b.id, "east", &c.id)?;

        // Path from a to c exists
        assert!(db.has_path(&a.id, &c.id)?);

        // No path from c to a (one-way)
        assert!(!db.has_path(&c.id, &a.id)?);

        Ok(())
    }

    #[test]
    fn test_direction_helpers() {
        assert_eq!(opposite_direction("north"), Some("south"));
        assert_eq!(opposite_direction("n"), Some("south"));
        assert_eq!(opposite_direction("custom"), None);

        assert_eq!(normalize_direction("n"), "north");
        assert_eq!(normalize_direction("custom"), "custom");
    }
}
