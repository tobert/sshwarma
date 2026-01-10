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
    // Lua code fields (for executable things)
    pub code: Option<String>,        // Lua source code
    pub default_slot: Option<String>, // Default slot: 'command:look', NULL, etc.
    pub params: Option<String>,      // JSON parameter schema
    // Status
    pub available: bool,
    // Lifecycle
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    pub created_by: Option<String>,  // Agent who created this thing
    pub copied_from: Option<String>, // Source thing ID for CoW copies
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
            code: None,
            default_slot: None,
            params: None,
            available: true,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            created_by: None,
            copied_from: None,
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
                content, uri, metadata, code, default_slot, params,
                available, created_at, updated_at, deleted_at, created_by, copied_from)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"#,
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
                thing.code,
                thing.default_slot,
                thing.params,
                thing.available,
                thing.created_at,
                thing.updated_at,
                thing.deleted_at,
                thing.created_by,
                thing.copied_from,
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
                      content, uri, metadata, code, default_slot, params,
                      available, created_at, updated_at, deleted_at, created_by, copied_from
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
                      content, uri, metadata, code, default_slot, params,
                      available, created_at, updated_at, deleted_at, created_by, copied_from
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
                      content, uri, metadata, code, default_slot, params,
                      available, created_at, updated_at, deleted_at, created_by, copied_from
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
                      content, uri, metadata, code, default_slot, params,
                      available, created_at, updated_at, deleted_at, created_by, copied_from
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
                      content, uri, metadata, code, default_slot, params,
                      available, created_at, updated_at, deleted_at, created_by, copied_from
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
                      content, uri, metadata, code, default_slot, params,
                      available, created_at, updated_at, deleted_at, created_by, copied_from
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
                      content, uri, metadata, code, default_slot, params,
                      available, created_at, updated_at, deleted_at, created_by, copied_from
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
               content = ?7, uri = ?8, metadata = ?9, code = ?10, default_slot = ?11,
               params = ?12, available = ?13, updated_at = ?14
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
                thing.code,
                thing.default_slot,
                thing.params,
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

    /// Copy a thing to a new parent (CoW)
    ///
    /// Creates a new thing with the same content but:
    /// - New ID
    /// - New parent_id
    /// - No qualified_name (copies don't get unique names)
    /// - Sets copied_from to track lineage
    pub fn copy_thing(&self, thing_id: &str, new_parent_id: &str) -> Result<Thing> {
        let original = self
            .get_thing(thing_id)?
            .ok_or_else(|| anyhow::anyhow!("thing not found: {}", thing_id))?;

        let now = now_ms();
        let copy = Thing {
            id: new_id(),
            parent_id: Some(new_parent_id.to_string()),
            kind: original.kind,
            name: original.name.clone(),
            qualified_name: None, // Copies don't get qualified names
            description: original.description.clone(),
            content: original.content.clone(),
            uri: original.uri.clone(),
            metadata: original.metadata.clone(),
            code: original.code.clone(),
            default_slot: original.default_slot.clone(),
            params: original.params.clone(),
            available: original.available,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            created_by: original.created_by.clone(),
            copied_from: Some(thing_id.to_string()),
        };

        self.insert_thing(&copy)?;
        tracing::debug!(
            original = thing_id,
            copy = %copy.id,
            parent = new_parent_id,
            "copied thing"
        );

        Ok(copy)
    }

    /// Move a thing to a new parent
    pub fn move_thing(&self, thing_id: &str, new_parent_id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE things SET parent_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![thing_id, new_parent_id, now_ms()],
        )
        .context("failed to move thing")?;
        tracing::debug!(thing = thing_id, new_parent = new_parent_id, "moved thing");
        Ok(())
    }

    /// Ensure an agent has a corresponding Thing in the world tree.
    ///
    /// Creates a thing with id "agent_{name}" under the agents/ container
    /// if it doesn't already exist. This allows agents to own things directly.
    pub fn ensure_agent_thing(&self, agent_name: &str) -> Result<String> {
        let thing_id = format!("agent_{}", agent_name);

        // Check if already exists
        if self.get_thing(&thing_id)?.is_some() {
            return Ok(thing_id);
        }

        // Create agent thing under agents/ container
        let now = now_ms();
        let thing = Thing {
            id: thing_id.clone(),
            parent_id: Some(ids::AGENTS.to_string()),
            kind: ThingKind::Agent,
            name: agent_name.to_string(),
            qualified_name: None,
            description: Some(format!("Agent: {}", agent_name)),
            content: None,
            uri: None,
            metadata: None,
            code: None,
            default_slot: None,
            params: None,
            available: true,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            created_by: None,
            copied_from: None,
        };

        self.insert_thing(&thing)?;
        tracing::info!(agent = agent_name, thing_id = %thing_id, "created agent thing");

        Ok(thing_id)
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
                content, uri, metadata, code, default_slot, params,
                available, created_at, updated_at, deleted_at, created_by, copied_from)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
               ON CONFLICT(qualified_name) WHERE deleted_at IS NULL AND qualified_name IS NOT NULL
               DO UPDATE SET
                   description = excluded.description,
                   metadata = excluded.metadata,
                   code = excluded.code,
                   default_slot = excluded.default_slot,
                   params = excluded.params,
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
                thing.code,
                thing.default_slot,
                thing.params,
                thing.available,
                thing.created_at,
                thing.updated_at,
                thing.deleted_at,
                thing.created_by,
                thing.copied_from,
            ],
        )
        .context("failed to upsert thing")?;
        Ok(())
    }

    // Helper to parse a row into a Thing
    // Expects columns in order:
    // 0: id, 1: parent_id, 2: kind, 3: name, 4: qualified_name, 5: description,
    // 6: content, 7: uri, 8: metadata, 9: code, 10: default_slot, 11: params,
    // 12: available, 13: created_at, 14: updated_at, 15: deleted_at, 16: created_by, 17: copied_from
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
            code: row.get(9)?,
            default_slot: row.get(10)?,
            params: row.get(11)?,
            available: row.get(12)?,
            created_at: row.get(13)?,
            updated_at: row.get(14)?,
            deleted_at: row.get(15)?,
            created_by: row.get(16)?,
            copied_from: row.get(17)?,
        })
    }

    /// Bootstrap the world structure if it doesn't exist.
    /// Creates:
    /// - world (root container)
    /// - rooms, agents, mcps, internal, defaults, shared (top-level containers)
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
            ("shared", "System resources accessible to all"),
        ];

        for (name, desc) in containers {
            let mut container = Thing::container(name).with_parent("world");
            container.id = name.to_string();
            container.description = Some(desc.to_string());
            self.insert_thing(&container)?;
        }

        // Create home room (shared resources)
        // Thing entry for world tree
        let mut home = Thing::room("home").with_parent("rooms");
        home.id = "home".to_string();
        home.description = Some("Shared resources accessible from all rooms".to_string());
        home.metadata = Some(r#"{"vibe": "Shared resources"}"#.to_string());
        self.insert_thing(&home)?;
        // Room entry for equipment/sessions
        let mut home_room = super::rooms::Room::new("home");
        home_room.id = "home".to_string();
        self.insert_room(&home_room)?;

        // Create lobby room (default landing)
        // Thing entry for world tree
        let mut lobby = Thing::room("lobby").with_parent("rooms");
        lobby.id = "lobby".to_string();
        lobby.description = Some("Welcome to sshwarma".to_string());
        lobby.metadata = Some(r#"{"vibe": "Welcome to sshwarma"}"#.to_string());
        self.insert_thing(&lobby)?;
        // Room entry for equipment/sessions
        let mut lobby_room = super::rooms::Room::new("lobby");
        lobby_room.id = "lobby".to_string();
        self.insert_room(&lobby_room)?;

        // Register internal tools with Lua code
        // Format: (name, qualified_name, description, code, default_slot)
        let internal_tools: Vec<(&str, &str, &str, &str, Option<&str>)> = vec![
            // Core observation tools (no slot - always available to LLM)
            (
                "look",
                "sshwarma:look",
                "Describe current room",
                include_str!("../embedded/tools/look.lua"),
                None,
            ),
            (
                "who",
                "sshwarma:who",
                "List participants in room",
                include_str!("../embedded/tools/who.lua"),
                None,
            ),
            (
                "rooms",
                "sshwarma:rooms",
                "List available rooms",
                include_str!("../embedded/tools/rooms.lua"),
                None,
            ),
            (
                "history",
                "sshwarma:history",
                "View conversation history",
                include_str!("../embedded/tools/history.lua"),
                None,
            ),
            (
                "exits",
                "sshwarma:exits",
                "List room exits",
                include_str!("../embedded/tools/exits.lua"),
                None,
            ),
            // Write tools (no slot - available to LLM)
            (
                "say",
                "sshwarma:say",
                "Send message to room",
                include_str!("../embedded/tools/say.lua"),
                None,
            ),
            (
                "vibe",
                "sshwarma:vibe",
                "Get or set room vibe",
                include_str!("../embedded/tools/vibe.lua"),
                None,
            ),
            // Navigation tools (no slot - available to LLM)
            (
                "join",
                "sshwarma:join",
                "Join a room",
                include_str!("../embedded/tools/join.lua"),
                None,
            ),
            (
                "leave",
                "sshwarma:leave",
                "Leave current room",
                include_str!("../embedded/tools/leave.lua"),
                None,
            ),
            (
                "go",
                "sshwarma:go",
                "Navigate through an exit",
                include_str!("../embedded/tools/go.lua"),
                None,
            ),
            (
                "create",
                "sshwarma:create",
                "Create a new room",
                include_str!("../embedded/tools/create.lua"),
                None,
            ),
            (
                "fork",
                "sshwarma:fork",
                "Fork current room with settings",
                include_str!("../embedded/tools/fork.lua"),
                None,
            ),
        ];

        let mut tool_ids = Vec::new();
        for (name, qualified, desc, code, default_slot) in internal_tools {
            let mut tool = Thing::tool(name, qualified)
                .with_parent("internal")
                .with_description(desc);
            tool.id = format!("tool_{}", name);
            tool.code = Some(code.to_string());
            tool.default_slot = default_slot.map(|s| s.to_string());
            self.insert_thing(&tool)?;
            tool_ids.push(tool.id);
        }

        // Equip internal tools directly to lobby room
        for (i, tool_id) in tool_ids.iter().enumerate() {
            self.room_equip("lobby", tool_id, None, None, i as f64)?;
        }

        tracing::info!(
            "bootstrapped world structure with {} internal tools",
            tool_ids.len()
        );
        Ok(())
    }

    /// Ensure the world structure exists (called on startup)
    pub fn ensure_world(&self) -> Result<()> {
        self.bootstrap_world()
    }

    /// Sync MCP tools to the things table
    ///
    /// Creates or updates tool things for each MCP tool, with qualified names
    /// like `holler:sample`. Tools are parented under an MCP container thing
    /// which is created if it doesn't exist.
    ///
    /// Tools that were previously synced but are no longer present are marked
    /// as unavailable (available=false) rather than deleted.
    pub fn sync_mcp_tools(
        &self,
        mcp_name: &str,
        tools: &[(String, String)], // (name, description)
    ) -> Result<usize> {
        // Ensure world is bootstrapped
        self.bootstrap_world()?;

        // Create or get MCP container thing (e.g., "holler" under "mcps")
        let mcp_thing_id = format!("mcp_{}", mcp_name);
        if self.get_thing(&mcp_thing_id)?.is_none() {
            let mut mcp_thing = Thing::mcp(mcp_name).with_parent(ids::MCPS);
            mcp_thing.id = mcp_thing_id.clone();
            mcp_thing.description = Some(format!("MCP server: {}", mcp_name));
            self.insert_thing(&mcp_thing)?;
            tracing::info!(mcp = %mcp_name, "created MCP container thing");
        }

        // Get existing tools for this MCP (by qualified_name prefix)
        let prefix = format!("{}:*", mcp_name);
        let existing = self.find_things_by_qualified_name(&prefix)?;

        // Track which tools we sync
        let mut synced_names = std::collections::HashSet::new();
        let mut count = 0;

        for (tool_name, description) in tools {
            let qualified_name = format!("{}:{}", mcp_name, tool_name);
            synced_names.insert(qualified_name.clone());

            // Check if tool already exists
            if let Some(existing_tool) = existing.iter().find(|t| {
                t.qualified_name.as_ref() == Some(&qualified_name)
            }) {
                // Update if description changed or if it was unavailable
                if existing_tool.description.as_ref() != Some(description)
                    || !existing_tool.available
                {
                    let mut updated = existing_tool.clone();
                    updated.description = Some(description.clone());
                    updated.available = true;
                    self.update_thing(&updated)?;
                    count += 1;
                }
            } else {
                // Create new tool thing
                let mut tool = Thing::tool(tool_name, &qualified_name)
                    .with_parent(&mcp_thing_id);
                tool.description = Some(description.clone());
                tool.available = true;
                self.insert_thing(&tool)?;
                count += 1;
            }
        }

        // Mark removed tools as unavailable
        for existing_tool in &existing {
            if let Some(ref qname) = existing_tool.qualified_name {
                if !synced_names.contains(qname) && existing_tool.available {
                    self.set_thing_available(&existing_tool.id, false)?;
                    tracing::debug!(tool = %qname, "marked MCP tool unavailable");
                }
            }
        }

        if count > 0 {
            tracing::info!(mcp = %mcp_name, synced = count, "synced MCP tools");
        }

        Ok(count)
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
    pub const SHARED: &str = "shared";
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
        assert_eq!(children.len(), 6); // rooms, agents, mcps, internal, defaults, shared

        // Verify rooms container has lobby and home
        let rooms = db.get_thing_children("rooms")?;
        assert_eq!(rooms.len(), 2);

        // Verify internal tools
        let tools = db.get_thing_children("internal")?;
        assert_eq!(tools.len(), 12); // 12 internal tools with Lua code

        // Verify lobby has equipped tools
        let equipped = db.get_room_equipment("lobby", None)?;
        assert_eq!(equipped.len(), 12);

        // Verify tools have Lua code
        let look = db.get_thing_by_qualified_name("sshwarma:look")?.unwrap();
        assert!(look.code.is_some());
        assert!(look.code.unwrap().contains("tools.look()"));

        // Verify sshwarma:look is registered
        let look = db.get_thing_by_qualified_name("sshwarma:look")?;
        assert!(look.is_some());

        // Bootstrap should be idempotent
        db.bootstrap_world()?;

        Ok(())
    }
}
