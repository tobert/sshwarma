//! View state storage
//!
//! Per-agent UI state including view stack and scroll positions.

use super::{new_id, now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// A view stack entry for a region
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewStack {
    pub id: String,
    pub agent_id: String,
    pub region_name: String,
    pub layers: Vec<serde_json::Value>, // JSON array of layer objects
    pub active_layer: i32,
    pub updated_at: i64,
}

impl ViewStack {
    /// Create a new view stack
    pub fn new(agent_id: impl Into<String>, region_name: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            agent_id: agent_id.into(),
            region_name: region_name.into(),
            layers: vec![],
            active_layer: 0,
            updated_at: now_ms(),
        }
    }

    /// Push a layer onto the stack
    pub fn push(&mut self, layer: serde_json::Value) {
        self.layers.push(layer);
        self.active_layer = (self.layers.len() - 1) as i32;
        self.updated_at = now_ms();
    }

    /// Pop the top layer
    pub fn pop(&mut self) -> Option<serde_json::Value> {
        if self.layers.is_empty() {
            return None;
        }
        let layer = self.layers.pop();
        self.active_layer = self.active_layer.saturating_sub(1);
        self.updated_at = now_ms();
        layer
    }

    /// Get the active layer
    pub fn active(&self) -> Option<&serde_json::Value> {
        self.layers.get(self.active_layer as usize)
    }
}

/// Scroll mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollMode {
    /// Follow new content (scroll to bottom)
    Tail,
    /// Stay at fixed position
    Pinned,
}

impl ScrollMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScrollMode::Tail => "tail",
            ScrollMode::Pinned => "pinned",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "tail" => Some(ScrollMode::Tail),
            "pinned" => Some(ScrollMode::Pinned),
            _ => None,
        }
    }
}

/// Scroll state for a buffer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferScroll {
    pub agent_id: String,
    pub buffer_id: String,
    pub scroll_row_id: Option<String>, // Row at top of viewport
    pub scroll_offset: i32,            // Offset within that row
    pub mode: ScrollMode,
    pub updated_at: i64,
}

impl BufferScroll {
    /// Create a new scroll state (defaults to tail mode)
    pub fn new(agent_id: impl Into<String>, buffer_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            buffer_id: buffer_id.into(),
            scroll_row_id: None,
            scroll_offset: 0,
            mode: ScrollMode::Tail,
            updated_at: now_ms(),
        }
    }

    /// Pin to a specific row
    pub fn pin_to(&mut self, row_id: impl Into<String>, offset: i32) {
        self.scroll_row_id = Some(row_id.into());
        self.scroll_offset = offset;
        self.mode = ScrollMode::Pinned;
        self.updated_at = now_ms();
    }

    /// Switch to tail mode
    pub fn tail(&mut self) {
        self.mode = ScrollMode::Tail;
        self.scroll_row_id = None;
        self.scroll_offset = 0;
        self.updated_at = now_ms();
    }
}

// Database operations
impl Database {
    // --- View Stack ---

    /// Get or create view stack for agent+region
    pub fn get_or_create_view_stack(
        &self,
        agent_id: &str,
        region_name: &str,
    ) -> Result<ViewStack> {
        if let Some(stack) = self.get_view_stack(agent_id, region_name)? {
            return Ok(stack);
        }

        let stack = ViewStack::new(agent_id, region_name);
        self.upsert_view_stack(&stack)?;
        Ok(stack)
    }

    /// Get view stack
    pub fn get_view_stack(&self, agent_id: &str, region_name: &str) -> Result<Option<ViewStack>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, agent_id, region_name, layers, active_layer, updated_at
            FROM view_stack WHERE agent_id = ?1 AND region_name = ?2
            "#,
            )
            .context("failed to prepare view stack query")?;

        let stack = stmt
            .query_row(params![agent_id, region_name], |row| {
                let layers_json: String = row.get(3)?;
                let layers: Vec<serde_json::Value> =
                    serde_json::from_str(&layers_json).unwrap_or_default();
                Ok(ViewStack {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    region_name: row.get(2)?,
                    layers,
                    active_layer: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .optional()
            .context("failed to query view stack")?;

        Ok(stack)
    }

    /// Insert or update view stack
    pub fn upsert_view_stack(&self, stack: &ViewStack) -> Result<()> {
        let conn = self.conn()?;
        let layers_json = serde_json::to_string(&stack.layers)?;

        conn.execute(
            r#"
            INSERT INTO view_stack (id, agent_id, region_name, layers, active_layer, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT (agent_id, region_name) DO UPDATE SET
                layers = ?4, active_layer = ?5, updated_at = ?6
            "#,
            params![
                stack.id,
                stack.agent_id,
                stack.region_name,
                layers_json,
                stack.active_layer,
                stack.updated_at,
            ],
        )
        .context("failed to upsert view stack")?;
        Ok(())
    }

    /// Delete view stack
    pub fn delete_view_stack(&self, agent_id: &str, region_name: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM view_stack WHERE agent_id = ?1 AND region_name = ?2",
            params![agent_id, region_name],
        )
        .context("failed to delete view stack")?;
        Ok(())
    }

    // --- Buffer Scroll ---

    /// Get or create scroll state for agent+buffer
    pub fn get_or_create_scroll(&self, agent_id: &str, buffer_id: &str) -> Result<BufferScroll> {
        if let Some(scroll) = self.get_scroll(agent_id, buffer_id)? {
            return Ok(scroll);
        }

        let scroll = BufferScroll::new(agent_id, buffer_id);
        self.upsert_scroll(&scroll)?;
        Ok(scroll)
    }

    /// Get scroll state
    pub fn get_scroll(&self, agent_id: &str, buffer_id: &str) -> Result<Option<BufferScroll>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT agent_id, buffer_id, scroll_row_id, scroll_offset, mode, updated_at
            FROM buffer_scroll WHERE agent_id = ?1 AND buffer_id = ?2
            "#,
            )
            .context("failed to prepare scroll query")?;

        let scroll = stmt
            .query_row(params![agent_id, buffer_id], |row| {
                let mode_str: String = row.get(4)?;
                Ok(BufferScroll {
                    agent_id: row.get(0)?,
                    buffer_id: row.get(1)?,
                    scroll_row_id: row.get(2)?,
                    scroll_offset: row.get(3)?,
                    mode: ScrollMode::from_str(&mode_str).unwrap_or(ScrollMode::Tail),
                    updated_at: row.get(5)?,
                })
            })
            .optional()
            .context("failed to query scroll")?;

        Ok(scroll)
    }

    /// Insert or update scroll state
    pub fn upsert_scroll(&self, scroll: &BufferScroll) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO buffer_scroll (agent_id, buffer_id, scroll_row_id, scroll_offset, mode, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT (agent_id, buffer_id) DO UPDATE SET
                scroll_row_id = ?3, scroll_offset = ?4, mode = ?5, updated_at = ?6
            "#,
            params![
                scroll.agent_id,
                scroll.buffer_id,
                scroll.scroll_row_id,
                scroll.scroll_offset,
                scroll.mode.as_str(),
                scroll.updated_at,
            ],
        )
        .context("failed to upsert scroll")?;
        Ok(())
    }

    /// Delete scroll state
    pub fn delete_scroll(&self, agent_id: &str, buffer_id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM buffer_scroll WHERE agent_id = ?1 AND buffer_id = ?2",
            params![agent_id, buffer_id],
        )
        .context("failed to delete scroll")?;
        Ok(())
    }

    /// List all scroll states for an agent
    pub fn list_agent_scrolls(&self, agent_id: &str) -> Result<Vec<BufferScroll>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT agent_id, buffer_id, scroll_row_id, scroll_offset, mode, updated_at
            FROM buffer_scroll WHERE agent_id = ?1
            "#,
            )
            .context("failed to prepare scrolls query")?;

        let scrolls = stmt
            .query(params![agent_id])?
            .mapped(|row| {
                let mode_str: String = row.get(4)?;
                Ok(BufferScroll {
                    agent_id: row.get(0)?,
                    buffer_id: row.get(1)?,
                    scroll_row_id: row.get(2)?,
                    scroll_offset: row.get(3)?,
                    mode: ScrollMode::from_str(&mode_str).unwrap_or(ScrollMode::Tail),
                    updated_at: row.get(5)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list scrolls")?;

        Ok(scrolls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{agents::Agent, agents::AgentKind, buffers::Buffer, rooms::Room};

    fn setup() -> Result<(Database, String, String)> {
        let db = Database::in_memory()?;

        let agent = Agent::new("test_agent", AgentKind::Human);
        db.insert_agent(&agent)?;

        let room = Room::new("test_room");
        db.insert_room(&room)?;

        let buffer = Buffer::room_chat(&room.id);
        db.insert_buffer(&buffer)?;

        Ok((db, agent.id, buffer.id))
    }

    #[test]
    fn test_view_stack_crud() -> Result<()> {
        let (db, agent_id, _buffer_id) = setup()?;

        let mut stack = ViewStack::new(&agent_id, "main");
        stack.push(serde_json::json!({"type": "chat", "buffer_id": "buf1"}));
        stack.push(serde_json::json!({"type": "help", "topic": "commands"}));

        db.upsert_view_stack(&stack)?;

        let fetched = db
            .get_view_stack(&agent_id, "main")?
            .expect("stack should exist");
        assert_eq!(fetched.layers.len(), 2);
        assert_eq!(fetched.active_layer, 1);

        // Pop and update
        stack.pop();
        db.upsert_view_stack(&stack)?;

        let fetched = db.get_view_stack(&agent_id, "main")?.unwrap();
        assert_eq!(fetched.layers.len(), 1);
        assert_eq!(fetched.active_layer, 0);

        db.delete_view_stack(&agent_id, "main")?;
        assert!(db.get_view_stack(&agent_id, "main")?.is_none());

        Ok(())
    }

    #[test]
    fn test_view_stack_get_or_create() -> Result<()> {
        let (db, agent_id, _buffer_id) = setup()?;

        // First call creates
        let stack1 = db.get_or_create_view_stack(&agent_id, "sidebar")?;
        assert!(stack1.layers.is_empty());

        // Second call returns existing
        let stack2 = db.get_or_create_view_stack(&agent_id, "sidebar")?;
        assert_eq!(stack1.id, stack2.id);

        Ok(())
    }

    #[test]
    fn test_scroll_crud() -> Result<()> {
        let (db, agent_id, buffer_id) = setup()?;

        let mut scroll = BufferScroll::new(&agent_id, &buffer_id);
        db.upsert_scroll(&scroll)?;

        let fetched = db
            .get_scroll(&agent_id, &buffer_id)?
            .expect("scroll should exist");
        assert_eq!(fetched.mode, ScrollMode::Tail);
        assert!(fetched.scroll_row_id.is_none());

        // Pin to a row
        scroll.pin_to("row123", 5);
        db.upsert_scroll(&scroll)?;

        let fetched = db.get_scroll(&agent_id, &buffer_id)?.unwrap();
        assert_eq!(fetched.mode, ScrollMode::Pinned);
        assert_eq!(fetched.scroll_row_id, Some("row123".to_string()));
        assert_eq!(fetched.scroll_offset, 5);

        // Back to tail
        scroll.tail();
        db.upsert_scroll(&scroll)?;

        let fetched = db.get_scroll(&agent_id, &buffer_id)?.unwrap();
        assert_eq!(fetched.mode, ScrollMode::Tail);
        assert!(fetched.scroll_row_id.is_none());

        db.delete_scroll(&agent_id, &buffer_id)?;
        assert!(db.get_scroll(&agent_id, &buffer_id)?.is_none());

        Ok(())
    }

    #[test]
    fn test_scroll_get_or_create() -> Result<()> {
        let (db, agent_id, buffer_id) = setup()?;

        // First call creates with tail mode
        let scroll1 = db.get_or_create_scroll(&agent_id, &buffer_id)?;
        assert_eq!(scroll1.mode, ScrollMode::Tail);

        // Second call returns existing
        let scroll2 = db.get_or_create_scroll(&agent_id, &buffer_id)?;
        assert_eq!(scroll1.buffer_id, scroll2.buffer_id);

        Ok(())
    }

    #[test]
    fn test_list_agent_scrolls() -> Result<()> {
        let (db, agent_id, buffer_id) = setup()?;

        // Create a second buffer
        let room = Room::new("room2");
        db.insert_room(&room)?;
        let buffer2 = Buffer::room_chat(&room.id);
        db.insert_buffer(&buffer2)?;

        let scroll1 = BufferScroll::new(&agent_id, &buffer_id);
        let scroll2 = BufferScroll::new(&agent_id, &buffer2.id);

        db.upsert_scroll(&scroll1)?;
        db.upsert_scroll(&scroll2)?;

        let scrolls = db.list_agent_scrolls(&agent_id)?;
        assert_eq!(scrolls.len(), 2);

        Ok(())
    }

    #[test]
    fn test_view_stack_operations() {
        let mut stack = ViewStack::new("agent1", "main");

        // Empty stack
        assert!(stack.active().is_none());
        assert!(stack.pop().is_none());

        // Push layers
        stack.push(serde_json::json!({"layer": 1}));
        assert_eq!(stack.active_layer, 0);
        assert!(stack.active().is_some());

        stack.push(serde_json::json!({"layer": 2}));
        assert_eq!(stack.active_layer, 1);

        // Pop
        let popped = stack.pop().unwrap();
        assert_eq!(popped["layer"], 2);
        assert_eq!(stack.active_layer, 0);

        let popped = stack.pop().unwrap();
        assert_eq!(popped["layer"], 1);
        assert!(stack.active().is_none());
    }
}
