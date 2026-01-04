//! Things CRUD operations
//!
//! Things are the universal nodes in sshwarma's world tree.
//! Everything is a thing: rooms, agents, MCPs, tools, data, references.

use super::{new_id, now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Thing kinds in the world tree
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThingKind {
    /// Structural grouping (world, rooms, agents, mcps, internal, defaults)
    Container,
    /// Collaboration space
    Room,
    /// Actor (human, model, bot)
    Agent,
    /// External tool provider (MCP server)
    Mcp,
    /// Invokable capability
    Tool,
    /// Inline content (prompts, notes)
    Data,
    /// URI to external resource
    Reference,
}

impl ThingKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Room => "room",
            Self::Agent => "agent",
            Self::Mcp => "mcp",
            Self::Tool => "tool",
            Self::Data => "data",
            Self::Reference => "reference",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "container" => Some(Self::Container),
            "room" => Some(Self::Room),
            "agent" => Some(Self::Agent),
            "mcp" => Some(Self::Mcp),
            "tool" => Some(Self::Tool),
            "data" => Some(Self::Data),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }
}

/// A thing in the world tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thing {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: ThingKind,
    pub name: String,
    pub qualified_name: Option<String>,
    pub description: Option<String>,
    pub content: Option<String>,
    pub uri: Option<String>,
    pub metadata: Option<String>,
    pub available: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

impl Thing {
    /// Create a new thing with minimal fields
    pub fn new(name: impl Into<String>, kind: ThingKind) -> Self {
        let now = now_ms();
        Self {
            id: new_id(),
            parent_id: None,
            kind,
            name: name.into(),
            qualified_name: None,
            description: None,
            content: None,
            uri: None,
            metadata: None,
            available: true,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    /// Create a container thing
    pub fn container(name: impl Into<String>) -> Self {
        Self::new(name, ThingKind::Container)
    }

    /// Create a room thing
    pub fn room(name: impl Into<String>) -> Self {
        Self::new(name, ThingKind::Room)
    }

    /// Create an agent thing
    pub fn agent(name: impl Into<String>) -> Self {
        Self::new(name, ThingKind::Agent)
    }

    /// Create an MCP thing
    pub fn mcp(name: impl Into<String>) -> Self {
        Self::new(name, ThingKind::Mcp)
    }

    /// Create a tool thing with qualified name
    pub fn tool(name: impl Into<String>, qualified: impl Into<String>) -> Self {
        let mut thing = Self::new(name, ThingKind::Tool);
        thing.qualified_name = Some(qualified.into());
        thing
    }

    /// Create a data thing (inline content)
    pub fn data(name: impl Into<String>, content: impl Into<String>) -> Self {
        let mut thing = Self::new(name, ThingKind::Data);
        thing.content = Some(content.into());
        thing
    }

    /// Create a reference thing (external URI)
    pub fn reference(name: impl Into<String>, uri: impl Into<String>) -> Self {
        let mut thing = Self::new(name, ThingKind::Reference);
        thing.uri = Some(uri.into());
        thing
    }

    /// Set parent (builder pattern)
    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    /// Set description (builder pattern)
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set metadata JSON (builder pattern)
    pub fn with_metadata(mut self, metadata: impl Into<String>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }

    /// Check if this thing is deleted
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

// =============================================================================
// Database operations
// =============================================================================

impl Database {
    /// Insert a new thing
    pub fn insert_thing(&self, thing: &Thing) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO things
               (id, parent_id, kind, name, qualified_name, description,
                content, uri, metadata, available, created_at, updated_at, deleted_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
            params![
                thing.id,
                thing.parent_id,
                thing.kind.as_str(),
                thing.name,
                thing.qualified_name,
                thing.description,
                thing.content,
                thing.uri,
                thing.metadata,
                thing.available,
                thing.created_at,
                thing.updated_at,
                thing.deleted_at,
            ],
        )
        .context("failed to insert thing")?;
        Ok(())
    }

    /// Get a thing by ID
    pub fn get_thing(&self, id: &str) -> Result<Option<Thing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT id, parent_id, kind, name, qualified_name, description,
                      content, uri, metadata, available, created_at, updated_at, deleted_at
               FROM things WHERE id = ?1"#,
        )?;
        stmt.query_row(params![id], Self::thing_from_row)
            .optional()
            .context("failed to get thing")
    }

    /// Get a thing by qualified name (unique, not deleted)
    pub fn get_thing_by_qualified_name(&self, qualified_name: &str) -> Result<Option<Thing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT id, parent_id, kind, name, qualified_name, description,
                      content, uri, metadata, available, created_at, updated_at, deleted_at
               FROM things
               WHERE qualified_name = ?1 AND deleted_at IS NULL"#,
        )?;
        stmt.query_row(params![qualified_name], Self::thing_from_row)
            .optional()
            .context("failed to get thing by qualified name")
    }

    /// Get children of a thing (not deleted)
    pub fn get_thing_children(&self, parent_id: &str) -> Result<Vec<Thing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT id, parent_id, kind, name, qualified_name, description,
                      content, uri, metadata, available, created_at, updated_at, deleted_at
               FROM things
               WHERE parent_id = ?1 AND deleted_at IS NULL
               ORDER BY name"#,
        )?;
        let rows = stmt.query_map(params![parent_id], Self::thing_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get thing children")
    }

    /// Get children of a thing filtered by kind
    pub fn get_thing_children_by_kind(
        &self,
        parent_id: &str,
        kind: ThingKind,
    ) -> Result<Vec<Thing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT id, parent_id, kind, name, qualified_name, description,
                      content, uri, metadata, available, created_at, updated_at, deleted_at
               FROM things
               WHERE parent_id = ?1 AND kind = ?2 AND deleted_at IS NULL
               ORDER BY name"#,
        )?;
        let rows = stmt.query_map(params![parent_id, kind.as_str()], Self::thing_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to get thing children by kind")
    }

    /// List all things of a kind (not deleted)
    pub fn list_things_by_kind(&self, kind: ThingKind) -> Result<Vec<Thing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT id, parent_id, kind, name, qualified_name, description,
                      content, uri, metadata, available, created_at, updated_at, deleted_at
               FROM things
               WHERE kind = ?1 AND deleted_at IS NULL
               ORDER BY name"#,
        )?;
        let rows = stmt.query_map(params![kind.as_str()], Self::thing_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to list things by kind")
    }

    /// Find things by name pattern (glob matching, not deleted)
    pub fn find_things_by_name(&self, pattern: &str) -> Result<Vec<Thing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT id, parent_id, kind, name, qualified_name, description,
                      content, uri, metadata, available, created_at, updated_at, deleted_at
               FROM things
               WHERE name GLOB ?1 AND deleted_at IS NULL
               ORDER BY name"#,
        )?;
        let rows = stmt.query_map(params![pattern], Self::thing_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to find things by name")
    }

    /// Find things by qualified name pattern (glob matching, not deleted)
    pub fn find_things_by_qualified_name(&self, pattern: &str) -> Result<Vec<Thing>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT id, parent_id, kind, name, qualified_name, description,
                      content, uri, metadata, available, created_at, updated_at, deleted_at
               FROM things
               WHERE qualified_name GLOB ?1 AND deleted_at IS NULL
               ORDER BY qualified_name"#,
        )?;
        let rows = stmt.query_map(params![pattern], Self::thing_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("failed to find things by qualified name")
    }

    /// Update a thing (sets updated_at automatically)
    pub fn update_thing(&self, thing: &Thing) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"UPDATE things SET
               parent_id = ?2, kind = ?3, name = ?4, qualified_name = ?5, description = ?6,
               content = ?7, uri = ?8, metadata = ?9, available = ?10, updated_at = ?11
               WHERE id = ?1"#,
            params![
                thing.id,
                thing.parent_id,
                thing.kind.as_str(),
                thing.name,
                thing.qualified_name,
                thing.description,
                thing.content,
                thing.uri,
                thing.metadata,
                thing.available,
                now_ms(),
            ],
        )
        .context("failed to update thing")?;
        Ok(())
    }

    /// Soft-delete a thing (sets deleted_at)
    pub fn soft_delete_thing(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE things SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1",
            params![id, now_ms()],
        )
        .context("failed to soft-delete thing")?;
        Ok(())
    }

    /// Restore a soft-deleted thing
    pub fn restore_thing(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE things SET deleted_at = NULL, updated_at = ?2 WHERE id = ?1",
            params![id, now_ms()],
        )
        .context("failed to restore thing")?;
        Ok(())
    }

    /// Set thing availability (for MCP tools)
    pub fn set_thing_available(&self, id: &str, available: bool) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE things SET available = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, available, now_ms()],
        )
        .context("failed to set thing available")?;
        Ok(())
    }

    /// Hard-delete a thing (permanent, for garbage collection)
    pub fn hard_delete_thing(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM things WHERE id = ?1", params![id])
            .context("failed to hard-delete thing")?;
        Ok(())
    }

    /// Get the root thing (world container, id = 'world')
    pub fn get_world(&self) -> Result<Option<Thing>> {
        self.get_thing("world")
    }

    /// Upsert a thing by qualified name (for MCP tool sync)
    pub fn upsert_thing_by_qualified_name(&self, thing: &Thing) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO things
               (id, parent_id, kind, name, qualified_name, description,
                content, uri, metadata, available, created_at, updated_at, deleted_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
               ON CONFLICT(qualified_name) WHERE deleted_at IS NULL AND qualified_name IS NOT NULL
               DO UPDATE SET
                   description = excluded.description,
                   metadata = excluded.metadata,
                   available = excluded.available,
                   updated_at = excluded.updated_at"#,
            params![
                thing.id,
                thing.parent_id,
                thing.kind.as_str(),
                thing.name,
                thing.qualified_name,
                thing.description,
                thing.content,
                thing.uri,
                thing.metadata,
                thing.available,
                thing.created_at,
                thing.updated_at,
                thing.deleted_at,
            ],
        )
        .context("failed to upsert thing")?;
        Ok(())
    }

    // Helper to parse a row into a Thing
    fn thing_from_row(row: &rusqlite::Row) -> rusqlite::Result<Thing> {
        let kind_str: String = row.get(2)?;
        let kind = ThingKind::parse(&kind_str).unwrap_or(ThingKind::Data);
        Ok(Thing {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            kind,
            name: row.get(3)?,
            qualified_name: row.get(4)?,
            description: row.get(5)?,
            content: row.get(6)?,
            uri: row.get(7)?,
            metadata: row.get(8)?,
            available: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
            deleted_at: row.get(12)?,
        })
    }

    /// Bootstrap the world structure if it doesn't exist.
    /// Creates:
    /// - world (root container)
    /// - rooms, agents, mcps, internal, defaults (top-level containers)
    /// - home (shared resources room)
    /// - lobby (default landing room)
    /// - Internal tools (sshwarma:look, sshwarma:say, etc.)
    /// - Default equipped relationships
    pub fn bootstrap_world(&self) -> Result<()> {
        // Check if world already exists
        if self.get_thing("world")?.is_some() {
            return Ok(());
        }

        // Create world (use fixed ID for stability)
        let mut world = Thing::container("world");
        world.id = "world".to_string();
        world.description = Some("Root of the world tree".to_string());
        self.insert_thing(&world)?;

        // Create top-level containers with fixed IDs
        let containers = [
            ("rooms", "Container for collaboration spaces"),
            ("agents", "Container for humans, models, and bots"),
            ("mcps", "Container for MCP server connections"),
            ("internal", "Container for internal sshwarma tools"),
            ("defaults", "Default equipped relationships for new rooms"),
        ];

        for (name, desc) in containers {
            let mut container = Thing::container(name).with_parent("world");
            container.id = name.to_string();
            container.description = Some(desc.to_string());
            self.insert_thing(&container)?;
        }

        // Create home room (shared resources)
        let mut home = Thing::room("home").with_parent("rooms");
        home.id = "home".to_string();
        home.description = Some("Shared resources accessible from all rooms".to_string());
        home.metadata = Some(r#"{"vibe": "Shared resources"}"#.to_string());
        self.insert_thing(&home)?;

        // Create lobby room (default landing)
        let mut lobby = Thing::room("lobby").with_parent("rooms");
        lobby.id = "lobby".to_string();
        lobby.description = Some("Welcome to sshwarma".to_string());
        lobby.metadata = Some(r#"{"vibe": "Welcome to sshwarma"}"#.to_string());
        self.insert_thing(&lobby)?;

        // Register internal tools
        let internal_tools = [
            // Core
            ("look", "sshwarma:look", "Describe current room"),
            ("who", "sshwarma:who", "List participants in room"),
            ("say", "sshwarma:say", "Send message to room"),
            ("history", "sshwarma:history", "View conversation history"),
            ("vibe", "sshwarma:vibe", "Get or set room vibe"),
            ("inventory", "sshwarma:inventory", "Query room inventory"),
            // Journal
            ("journal", "sshwarma:journal", "Read journal entries"),
            ("note", "sshwarma:note", "Add a note to the journal"),
            ("decide", "sshwarma:decide", "Record a decision"),
            ("idea", "sshwarma:idea", "Capture an idea"),
            ("milestone", "sshwarma:milestone", "Mark a milestone"),
            // Navigation
            ("rooms", "sshwarma:rooms", "List available rooms"),
            ("exits", "sshwarma:exits", "List room exits"),
            ("join", "sshwarma:join", "Join a room"),
            ("leave", "sshwarma:leave", "Leave current room"),
            ("go", "sshwarma:go", "Navigate through an exit"),
            ("create", "sshwarma:create", "Create a new room"),
        ];

        let mut tool_ids = Vec::new();
        for (name, qualified, desc) in internal_tools {
            let mut tool = Thing::tool(name, qualified)
                .with_parent("internal")
                .with_description(desc);
            tool.id = format!("tool_{}", name);
            self.insert_thing(&tool)?;
            tool_ids.push(tool.id);
        }

        // Equip default tools in defaults container
        for (i, tool_id) in tool_ids.iter().enumerate() {
            self.equip("defaults", tool_id, i as f64)?;
        }

        // Copy equipped from defaults to lobby
        self.copy_equipped("defaults", "lobby")?;

        tracing::info!("bootstrapped world structure with {} internal tools", tool_ids.len());
        Ok(())
    }

    /// Ensure the world structure exists (called on startup)
    pub fn ensure_world(&self) -> Result<()> {
        self.bootstrap_world()
    }
}

// =============================================================================
// Well-known IDs for the world structure
// =============================================================================

/// Well-known thing IDs
pub mod ids {
    pub const WORLD: &str = "world";
    pub const ROOMS: &str = "rooms";
    pub const AGENTS: &str = "agents";
    pub const MCPS: &str = "mcps";
    pub const INTERNAL: &str = "internal";
    pub const DEFAULTS: &str = "defaults";
    pub const HOME: &str = "home";
    pub const LOBBY: &str = "lobby";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thing_crud() -> Result<()> {
        let db = Database::in_memory()?;

        // Create a container
        let world = Thing::container("world");
        db.insert_thing(&world)?;

        // Get it back
        let fetched = db.get_thing(&world.id)?.expect("world should exist");
        assert_eq!(fetched.name, "world");
        assert_eq!(fetched.kind, ThingKind::Container);

        // Create a child
        let rooms = Thing::container("rooms").with_parent(&world.id);
        db.insert_thing(&rooms)?;

        // Get children
        let children = db.get_thing_children(&world.id)?;
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "rooms");

        // Soft delete
        db.soft_delete_thing(&rooms.id)?;
        let children = db.get_thing_children(&world.id)?;
        assert_eq!(children.len(), 0);

        // Restore
        db.restore_thing(&rooms.id)?;
        let children = db.get_thing_children(&world.id)?;
        assert_eq!(children.len(), 1);

        Ok(())
    }

    #[test]
    fn test_thing_qualified_name() -> Result<()> {
        let db = Database::in_memory()?;

        // Create internal container
        let internal = Thing::container("internal");
        db.insert_thing(&internal)?;

        // Create a tool with qualified name
        let tool = Thing::tool("look", "sshwarma:look")
            .with_parent(&internal.id)
            .with_description("Describe current room");
        db.insert_thing(&tool)?;

        // Look up by qualified name
        let fetched = db
            .get_thing_by_qualified_name("sshwarma:look")?
            .expect("tool should exist");
        assert_eq!(fetched.name, "look");
        assert_eq!(fetched.description, Some("Describe current room".into()));

        // Find by pattern
        let tools = db.find_things_by_qualified_name("sshwarma:*")?;
        assert_eq!(tools.len(), 1);

        Ok(())
    }

    #[test]
    fn test_thing_availability() -> Result<()> {
        let db = Database::in_memory()?;

        let tool = Thing::tool("sample", "holler:sample");
        db.insert_thing(&tool)?;

        // Mark unavailable
        db.set_thing_available(&tool.id, false)?;
        let fetched = db.get_thing(&tool.id)?.unwrap();
        assert!(!fetched.available);

        // Mark available again
        db.set_thing_available(&tool.id, true)?;
        let fetched = db.get_thing(&tool.id)?.unwrap();
        assert!(fetched.available);

        Ok(())
    }

    #[test]
    fn test_bootstrap_world() -> Result<()> {
        let db = Database::in_memory()?;

        // Bootstrap should succeed
        db.bootstrap_world()?;

        // Verify world structure
        let world = db.get_thing("world")?.expect("world should exist");
        assert_eq!(world.kind, ThingKind::Container);

        // Verify containers
        let children = db.get_thing_children("world")?;
        assert_eq!(children.len(), 5); // rooms, agents, mcps, internal, defaults

        // Verify rooms container has lobby and home
        let rooms = db.get_thing_children("rooms")?;
        assert_eq!(rooms.len(), 2);

        // Verify internal tools
        let tools = db.get_thing_children("internal")?;
        assert!(tools.len() >= 17); // At least 17 internal tools

        // Verify lobby has equipped tools (copied from defaults)
        let equipped = db.get_equipped("lobby")?;
        assert!(equipped.len() >= 17);

        // Verify sshwarma:look is registered
        let look = db.get_thing_by_qualified_name("sshwarma:look")?;
        assert!(look.is_some());

        // Bootstrap should be idempotent
        db.bootstrap_world()?;

        Ok(())
    }
}
