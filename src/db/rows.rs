//! Row CRUD operations
//!
//! Rows are atomic units of content. Can nest via parent_row_id.
//! Uses fractional indexing for ordering within a buffer.

use super::{new_id, now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// A row in a buffer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub id: String,
    pub buffer_id: String,
    pub parent_row_id: Option<String>,
    pub position: f64,

    // Source
    pub source_agent_id: Option<String>,
    pub source_session_id: Option<String>,

    // Content
    pub content_method: String,
    pub content_format: String,
    pub content_meta: Option<String>,
    pub content: Option<String>,

    // Display state
    pub collapsed: bool,
    pub ephemeral: bool,
    pub mutable: bool,
    pub pinned: bool,
    pub hidden: bool,

    // Metrics
    pub token_count: Option<i32>,
    pub cost_usd: Option<f64>,
    pub latency_ms: Option<i32>,

    // Timestamps
    pub created_at: i64,
    pub updated_at: i64,
    pub finalized_at: Option<i64>,
}

impl Row {
    /// Create a new row
    pub fn new(buffer_id: impl Into<String>, content_method: impl Into<String>) -> Self {
        let now = now_ms();
        Self {
            id: new_id(),
            buffer_id: buffer_id.into(),
            parent_row_id: None,
            position: 0.0,
            source_agent_id: None,
            source_session_id: None,
            content_method: content_method.into(),
            content_format: "text".to_string(),
            content_meta: None,
            content: None,
            collapsed: false,
            ephemeral: false,
            mutable: false,
            pinned: false,
            hidden: false,
            token_count: None,
            cost_usd: None,
            latency_ms: None,
            created_at: now,
            updated_at: now,
            finalized_at: None,
        }
    }

    /// Create a message row
    pub fn message(
        buffer_id: impl Into<String>,
        agent_id: impl Into<String>,
        content: impl Into<String>,
        is_model: bool,
    ) -> Self {
        let method = if is_model {
            "message.model"
        } else {
            "message.user"
        };
        let mut row = Self::new(buffer_id, method);
        row.source_agent_id = Some(agent_id.into());
        row.content = Some(content.into());
        row.content_format = "markdown".to_string();
        row
    }

    /// Create a system message row
    pub fn system(buffer_id: impl Into<String>, content: impl Into<String>) -> Self {
        let mut row = Self::new(buffer_id, "message.system");
        row.content = Some(content.into());
        row
    }

    /// Create a thinking row
    pub fn thinking(buffer_id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        let mut row = Self::new(buffer_id, "thinking.stream");
        row.source_agent_id = Some(agent_id.into());
        row.mutable = true;
        row
    }

    /// Create a tool call row
    ///
    /// Represents an invocation of a tool by an agent.
    /// - `qualified_name`: The qualified tool name (e.g., "sshwarma:look", "holler:sample")
    /// - `input_json`: Optional JSON string of tool arguments
    pub fn tool_call(
        buffer_id: impl Into<String>,
        agent_id: impl Into<String>,
        qualified_name: impl Into<String>,
        input_json: Option<impl Into<String>>,
    ) -> Self {
        let qualified = qualified_name.into();
        let mut row = Self::new(buffer_id, "tool.call");
        row.source_agent_id = Some(agent_id.into());
        row.content = Some(qualified.clone());
        row.content_format = "json".to_string();
        // Store input in content_meta as JSON with tool info
        if let Some(input) = input_json {
            row.content_meta = Some(format!(
                r#"{{"tool":"{}","input":{}}}"#,
                qualified,
                input.into()
            ));
        } else {
            row.content_meta = Some(format!(r#"{{"tool":"{}"}}"#, qualified));
        }
        row.mutable = true; // Can be updated with result
        row
    }

    /// Create a tool result row
    ///
    /// Represents the result of a tool invocation.
    /// - `qualified_name`: The qualified tool name that produced this result
    /// - `result`: The tool's output (text or JSON)
    /// - `success`: Whether the tool call succeeded
    pub fn tool_result(
        buffer_id: impl Into<String>,
        qualified_name: impl Into<String>,
        result: impl Into<String>,
        success: bool,
    ) -> Self {
        let qualified = qualified_name.into();
        let mut row = Self::new(buffer_id, "tool.result");
        row.content = Some(result.into());
        row.content_format = "json".to_string();
        row.content_meta = Some(format!(
            r#"{{"tool":"{}","success":{}}}"#,
            qualified, success
        ));
        row
    }

    /// Create a tool call row that links to a parent row (the model message that invoked it)
    pub fn tool_call_with_parent(
        buffer_id: impl Into<String>,
        parent_row_id: impl Into<String>,
        agent_id: impl Into<String>,
        qualified_name: impl Into<String>,
        input_json: Option<impl Into<String>>,
    ) -> Self {
        let mut row = Self::tool_call(buffer_id, agent_id, qualified_name, input_json);
        row.parent_row_id = Some(parent_row_id.into());
        row
    }

    /// Finalize this row (marks it as complete)
    pub fn finalize(&mut self) {
        self.finalized_at = Some(now_ms());
        self.updated_at = now_ms();
        self.mutable = false;
    }
}

/// Tag on a row
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowTag {
    pub row_id: String,
    pub tag: String,
    pub created_at: i64,
}

/// Reaction on a row
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowReaction {
    pub id: String,
    pub row_id: String,
    pub agent_id: String,
    pub reaction: String,
    pub created_at: i64,
}

/// Link type discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkType {
    Reply,
    Quote,
    Relates,
    Continues,
}

impl LinkType {
    pub fn as_str(&self) -> &'static str {
        match self {
            LinkType::Reply => "reply",
            LinkType::Quote => "quote",
            LinkType::Relates => "relates",
            LinkType::Continues => "continues",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "reply" => Some(LinkType::Reply),
            "quote" => Some(LinkType::Quote),
            "relates" => Some(LinkType::Relates),
            "continues" => Some(LinkType::Continues),
            _ => None,
        }
    }
}

/// Link between rows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowLink {
    pub id: String,
    pub from_row_id: String,
    pub to_row_id: String,
    pub link_type: LinkType,
    pub created_at: i64,
}

// Fractional indexing helpers
pub mod fractional {
    /// Get a position between two values
    pub fn midpoint(a: f64, b: f64) -> f64 {
        (a + b) / 2.0
    }

    /// Get a position after the given value
    pub fn after(pos: f64) -> f64 {
        pos + 1.0
    }

    /// Get a position before the given value
    pub fn before(pos: f64) -> f64 {
        pos - 1.0
    }

    /// Check if positions are too close and need rebalancing
    pub fn needs_rebalance(a: f64, b: f64) -> bool {
        (b - a).abs() < 1e-10
    }
}

// Database operations
impl Database {
    /// Insert a new row
    pub fn insert_row(&self, row: &Row) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO rows (
                id, buffer_id, parent_row_id, position,
                source_agent_id, source_session_id,
                content_method, content_format, content_meta, content,
                collapsed, ephemeral, mutable, pinned, hidden,
                token_count, cost_usd, latency_ms,
                created_at, updated_at, finalized_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
            "#,
            params![
                row.id,
                row.buffer_id,
                row.parent_row_id,
                row.position,
                row.source_agent_id,
                row.source_session_id,
                row.content_method,
                row.content_format,
                row.content_meta,
                row.content,
                row.collapsed as i32,
                row.ephemeral as i32,
                row.mutable as i32,
                row.pinned as i32,
                row.hidden as i32,
                row.token_count,
                row.cost_usd,
                row.latency_ms,
                row.created_at,
                row.updated_at,
                row.finalized_at,
            ],
        )
        .context("failed to insert row")?;
        Ok(())
    }

    /// Get row by ID
    pub fn get_row(&self, id: &str) -> Result<Option<Row>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, buffer_id, parent_row_id, position,
                   source_agent_id, source_session_id,
                   content_method, content_format, content_meta, content,
                   collapsed, ephemeral, mutable, pinned, hidden,
                   token_count, cost_usd, latency_ms,
                   created_at, updated_at, finalized_at
            FROM rows WHERE id = ?1
            "#,
            )
            .context("failed to prepare row query")?;

        let row = stmt
            .query_row(params![id], Self::row_from_sqlite)
            .optional()
            .context("failed to query row")?;

        Ok(row)
    }

    /// List top-level rows in a buffer, ordered by position
    pub fn list_buffer_rows(&self, buffer_id: &str) -> Result<Vec<Row>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, buffer_id, parent_row_id, position,
                   source_agent_id, source_session_id,
                   content_method, content_format, content_meta, content,
                   collapsed, ephemeral, mutable, pinned, hidden,
                   token_count, cost_usd, latency_ms,
                   created_at, updated_at, finalized_at
            FROM rows
            WHERE buffer_id = ?1 AND parent_row_id IS NULL
            ORDER BY position
            "#,
            )
            .context("failed to prepare rows query")?;

        let rows = stmt
            .query(params![buffer_id])?
            .mapped(Self::row_from_sqlite)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list rows")?;

        Ok(rows)
    }

    /// List recent top-level rows in a buffer, ordered by position (most recent last)
    pub fn list_recent_buffer_rows(&self, buffer_id: &str, limit: usize) -> Result<Vec<Row>> {
        let conn = self.conn()?;
        // Get the last N rows by position (subquery to reverse order)
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, buffer_id, parent_row_id, position,
                   source_agent_id, source_session_id,
                   content_method, content_format, content_meta, content,
                   collapsed, ephemeral, mutable, pinned, hidden,
                   token_count, cost_usd, latency_ms,
                   created_at, updated_at, finalized_at
            FROM (
                SELECT * FROM rows
                WHERE buffer_id = ?1 AND parent_row_id IS NULL
                ORDER BY position DESC
                LIMIT ?2
            )
            ORDER BY position
            "#,
            )
            .context("failed to prepare recent rows query")?;

        let rows = stmt
            .query(params![buffer_id, limit as i64])?
            .mapped(Self::row_from_sqlite)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list recent rows")?;

        Ok(rows)
    }

    /// List child rows of a parent, ordered by position
    pub fn list_child_rows(&self, parent_row_id: &str) -> Result<Vec<Row>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, buffer_id, parent_row_id, position,
                   source_agent_id, source_session_id,
                   content_method, content_format, content_meta, content,
                   collapsed, ephemeral, mutable, pinned, hidden,
                   token_count, cost_usd, latency_ms,
                   created_at, updated_at, finalized_at
            FROM rows
            WHERE parent_row_id = ?1
            ORDER BY position
            "#,
            )
            .context("failed to prepare child rows query")?;

        let rows = stmt
            .query(params![parent_row_id])?
            .mapped(Self::row_from_sqlite)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list child rows")?;

        Ok(rows)
    }

    /// Get the last row in a buffer (for appending)
    pub fn get_last_buffer_row(&self, buffer_id: &str) -> Result<Option<Row>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, buffer_id, parent_row_id, position,
                   source_agent_id, source_session_id,
                   content_method, content_format, content_meta, content,
                   collapsed, ephemeral, mutable, pinned, hidden,
                   token_count, cost_usd, latency_ms,
                   created_at, updated_at, finalized_at
            FROM rows
            WHERE buffer_id = ?1 AND parent_row_id IS NULL
            ORDER BY position DESC
            LIMIT 1
            "#,
            )
            .context("failed to prepare last row query")?;

        let row = stmt
            .query_row(params![buffer_id], Self::row_from_sqlite)
            .optional()
            .context("failed to query last row")?;

        Ok(row)
    }

    /// Append a row to the end of a buffer
    pub fn append_row(&self, row: &mut Row) -> Result<()> {
        if let Some(last) = self.get_last_buffer_row(&row.buffer_id)? {
            row.position = fractional::after(last.position);
        } else {
            row.position = 0.0;
        }
        self.insert_row(row)
    }

    /// Update a row (for content streaming, finalization, etc.)
    pub fn update_row(&self, row: &Row) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE rows SET
                parent_row_id = ?2, position = ?3,
                source_agent_id = ?4, source_session_id = ?5,
                content_method = ?6, content_format = ?7, content_meta = ?8, content = ?9,
                collapsed = ?10, ephemeral = ?11, mutable = ?12, pinned = ?13, hidden = ?14,
                token_count = ?15, cost_usd = ?16, latency_ms = ?17,
                updated_at = ?18, finalized_at = ?19
            WHERE id = ?1
            "#,
            params![
                row.id,
                row.parent_row_id,
                row.position,
                row.source_agent_id,
                row.source_session_id,
                row.content_method,
                row.content_format,
                row.content_meta,
                row.content,
                row.collapsed as i32,
                row.ephemeral as i32,
                row.mutable as i32,
                row.pinned as i32,
                row.hidden as i32,
                row.token_count,
                row.cost_usd,
                row.latency_ms,
                row.updated_at,
                row.finalized_at,
            ],
        )
        .context("failed to update row")?;
        Ok(())
    }

    /// Delete a row (cascades to children, tags, reactions, links)
    pub fn delete_row(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM rows WHERE id = ?1", params![id])
            .context("failed to delete row")?;
        Ok(())
    }

    fn row_from_sqlite(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
        Ok(Row {
            id: r.get(0)?,
            buffer_id: r.get(1)?,
            parent_row_id: r.get(2)?,
            position: r.get(3)?,
            source_agent_id: r.get(4)?,
            source_session_id: r.get(5)?,
            content_method: r.get(6)?,
            content_format: r.get(7)?,
            content_meta: r.get(8)?,
            content: r.get(9)?,
            collapsed: r.get::<_, i32>(10)? != 0,
            ephemeral: r.get::<_, i32>(11)? != 0,
            mutable: r.get::<_, i32>(12)? != 0,
            pinned: r.get::<_, i32>(13)? != 0,
            hidden: r.get::<_, i32>(14)? != 0,
            token_count: r.get(15)?,
            cost_usd: r.get(16)?,
            latency_ms: r.get(17)?,
            created_at: r.get(18)?,
            updated_at: r.get(19)?,
            finalized_at: r.get(20)?,
        })
    }

    // --- Tags ---

    /// Add a tag to a row
    pub fn add_row_tag(&self, row_id: &str, tag: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO row_tags (row_id, tag, created_at) VALUES (?1, ?2, ?3)",
            params![row_id, tag, now_ms()],
        )
        .context("failed to add row tag")?;
        Ok(())
    }

    /// Remove a tag from a row
    pub fn remove_row_tag(&self, row_id: &str, tag: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM row_tags WHERE row_id = ?1 AND tag = ?2",
            params![row_id, tag],
        )
        .context("failed to remove row tag")?;
        Ok(())
    }

    /// Get all tags for a row
    pub fn get_row_tags(&self, row_id: &str) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT tag FROM row_tags WHERE row_id = ?1 ORDER BY tag")
            .context("failed to prepare tags query")?;

        let tags = stmt
            .query(params![row_id])?
            .mapped(|r| r.get(0))
            .collect::<Result<Vec<String>, _>>()
            .context("failed to list tags")?;

        Ok(tags)
    }

    /// Find rows with a specific tag in a buffer
    pub fn find_rows_by_tag(&self, buffer_id: &str, tag: &str) -> Result<Vec<Row>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT r.id, r.buffer_id, r.parent_row_id, r.position,
                   r.source_agent_id, r.source_session_id,
                   r.content_method, r.content_format, r.content_meta, r.content,
                   r.collapsed, r.ephemeral, r.mutable, r.pinned, r.hidden,
                   r.token_count, r.cost_usd, r.latency_ms,
                   r.created_at, r.updated_at, r.finalized_at
            FROM rows r
            JOIN row_tags rt ON r.id = rt.row_id
            WHERE r.buffer_id = ?1 AND rt.tag = ?2
            ORDER BY r.position
            "#,
            )
            .context("failed to prepare tagged rows query")?;

        let rows = stmt
            .query(params![buffer_id, tag])?
            .mapped(Self::row_from_sqlite)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to find rows by tag")?;

        Ok(rows)
    }

    // --- Reactions ---

    /// Add a reaction to a row
    pub fn add_row_reaction(&self, row_id: &str, agent_id: &str, reaction: &str) -> Result<String> {
        let conn = self.conn()?;
        let id = new_id();
        conn.execute(
            r#"
            INSERT INTO row_reactions (id, row_id, agent_id, reaction, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT (row_id, agent_id, reaction) DO NOTHING
            "#,
            params![id, row_id, agent_id, reaction, now_ms()],
        )
        .context("failed to add reaction")?;
        Ok(id)
    }

    /// Remove a reaction from a row
    pub fn remove_row_reaction(&self, row_id: &str, agent_id: &str, reaction: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM row_reactions WHERE row_id = ?1 AND agent_id = ?2 AND reaction = ?3",
            params![row_id, agent_id, reaction],
        )
        .context("failed to remove reaction")?;
        Ok(())
    }

    /// Get all reactions for a row
    pub fn get_row_reactions(&self, row_id: &str) -> Result<Vec<RowReaction>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, row_id, agent_id, reaction, created_at FROM row_reactions WHERE row_id = ?1 ORDER BY created_at",
            )
            .context("failed to prepare reactions query")?;

        let reactions = stmt
            .query(params![row_id])?
            .mapped(|r| {
                Ok(RowReaction {
                    id: r.get(0)?,
                    row_id: r.get(1)?,
                    agent_id: r.get(2)?,
                    reaction: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list reactions")?;

        Ok(reactions)
    }

    // --- Links ---

    /// Create a link between rows
    pub fn create_row_link(
        &self,
        from_row_id: &str,
        to_row_id: &str,
        link_type: LinkType,
    ) -> Result<String> {
        let conn = self.conn()?;
        let id = new_id();
        conn.execute(
            r#"
            INSERT INTO row_links (id, from_row_id, to_row_id, link_type, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![id, from_row_id, to_row_id, link_type.as_str(), now_ms()],
        )
        .context("failed to create row link")?;
        Ok(id)
    }

    /// Get outgoing links from a row
    pub fn get_row_links_from(&self, row_id: &str) -> Result<Vec<RowLink>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, from_row_id, to_row_id, link_type, created_at FROM row_links WHERE from_row_id = ?1",
            )
            .context("failed to prepare links query")?;

        let links = stmt
            .query(params![row_id])?
            .mapped(|r| {
                let link_type_str: String = r.get(3)?;
                Ok(RowLink {
                    id: r.get(0)?,
                    from_row_id: r.get(1)?,
                    to_row_id: r.get(2)?,
                    link_type: LinkType::parse(&link_type_str).unwrap_or(LinkType::Relates),
                    created_at: r.get(4)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list links from")?;

        Ok(links)
    }

    /// Get incoming links to a row
    pub fn get_row_links_to(&self, row_id: &str) -> Result<Vec<RowLink>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, from_row_id, to_row_id, link_type, created_at FROM row_links WHERE to_row_id = ?1",
            )
            .context("failed to prepare links query")?;

        let links = stmt
            .query(params![row_id])?
            .mapped(|r| {
                let link_type_str: String = r.get(3)?;
                Ok(RowLink {
                    id: r.get(0)?,
                    from_row_id: r.get(1)?,
                    to_row_id: r.get(2)?,
                    link_type: LinkType::parse(&link_type_str).unwrap_or(LinkType::Relates),
                    created_at: r.get(4)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list links to")?;

        Ok(links)
    }

    /// Delete a link
    pub fn delete_row_link(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM row_links WHERE id = ?1", params![id])
            .context("failed to delete row link")?;
        Ok(())
    }

    // --- Queries by content_method ---

    /// Append text to a row's content (for streaming)
    ///
    /// Used for streaming responses where content is incrementally added.
    /// Only works on mutable rows.
    pub fn append_to_row(&self, row_id: &str, text: &str) -> Result<()> {
        let conn = self.conn()?;
        let now = now_ms();
        conn.execute(
            r#"
            UPDATE rows SET
                content = COALESCE(content, '') || ?2,
                updated_at = ?3
            WHERE id = ?1 AND mutable = 1
            "#,
            params![row_id, text, now],
        )
        .context("failed to append to row")?;
        Ok(())
    }

    /// Finalize a row (mark as complete, set mutable=false)
    ///
    /// Used when streaming is complete or when a row should no longer be modified.
    pub fn finalize_row(&self, row_id: &str) -> Result<()> {
        let conn = self.conn()?;
        let now = now_ms();
        conn.execute(
            r#"
            UPDATE rows SET
                mutable = 0,
                finalized_at = ?2,
                updated_at = ?2
            WHERE id = ?1
            "#,
            params![row_id, now],
        )
        .context("failed to finalize row")?;
        Ok(())
    }

    /// Get rows since a specific row ID (for incremental rendering)
    ///
    /// Returns rows with position > the position of the given row.
    /// If since_id is None, returns all rows.
    pub fn rows_since(&self, buffer_id: &str, since_id: Option<&str>) -> Result<Vec<Row>> {
        let conn = self.conn()?;

        // Get the position of the since row
        let since_position: f64 = if let Some(id) = since_id {
            let mut stmt = conn
                .prepare("SELECT position FROM rows WHERE id = ?1")
                .context("failed to prepare position query")?;
            stmt.query_row(params![id], |r| r.get(0))
                .optional()
                .context("failed to get since row position")?
                .unwrap_or(-f64::INFINITY)
        } else {
            -f64::INFINITY
        };

        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, buffer_id, parent_row_id, position,
                   source_agent_id, source_session_id,
                   content_method, content_format, content_meta, content,
                   collapsed, ephemeral, mutable, pinned, hidden,
                   token_count, cost_usd, latency_ms,
                   created_at, updated_at, finalized_at
            FROM rows
            WHERE buffer_id = ?1 AND parent_row_id IS NULL AND position > ?2
            ORDER BY position
            "#,
            )
            .context("failed to prepare rows since query")?;

        let rows = stmt
            .query(params![buffer_id, since_position])?
            .mapped(Self::row_from_sqlite)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list rows since")?;

        Ok(rows)
    }

    /// List tool call rows for a buffer (tool.call and tool.result)
    /// Returns rows ordered by position (oldest first)
    pub fn list_tool_calls(&self, buffer_id: &str, limit: usize) -> Result<Vec<Row>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, buffer_id, parent_row_id, position,
                   source_agent_id, source_session_id,
                   content_method, content_format, content_meta, content,
                   collapsed, ephemeral, mutable, pinned, hidden,
                   token_count, cost_usd, latency_ms,
                   created_at, updated_at, finalized_at
            FROM rows
            WHERE buffer_id = ?1 AND content_method LIKE 'tool.%'
            ORDER BY position DESC
            LIMIT ?2
            "#,
            )
            .context("failed to prepare tool calls query")?;

        let rows = stmt
            .query(params![buffer_id, limit as i64])?
            .mapped(Self::row_from_sqlite)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list tool calls")?;

        // Return in chronological order (oldest first)
        let mut rows = rows;
        rows.reverse();
        Ok(rows)
    }

    /// Count tool calls by qualified name for a buffer
    /// Returns map of tool_name -> call_count
    pub fn count_tool_calls(
        &self,
        buffer_id: &str,
    ) -> Result<std::collections::HashMap<String, usize>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT content, COUNT(*) as cnt
            FROM rows
            WHERE buffer_id = ?1 AND content_method = 'tool.call'
            GROUP BY content
            ORDER BY cnt DESC
            "#,
        )?;

        let mut counts = std::collections::HashMap::new();
        let rows = stmt.query_map(params![buffer_id], |row| {
            let tool_name: Option<String> = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((tool_name.unwrap_or_default(), count as usize))
        })?;

        for row in rows {
            let (name, count) = row?;
            if !name.is_empty() {
                counts.insert(name, count);
            }
        }

        Ok(counts)
    }

    /// List rows by content method pattern (LIKE query)
    pub fn list_rows_by_method(&self, buffer_id: &str, method_pattern: &str) -> Result<Vec<Row>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, buffer_id, parent_row_id, position,
                   source_agent_id, source_session_id,
                   content_method, content_format, content_meta, content,
                   collapsed, ephemeral, mutable, pinned, hidden,
                   token_count, cost_usd, latency_ms,
                   created_at, updated_at, finalized_at
            FROM rows
            WHERE buffer_id = ?1 AND content_method LIKE ?2
            ORDER BY position
            "#,
            )
            .context("failed to prepare rows by method query")?;

        let rows = stmt
            .query(params![buffer_id, method_pattern])?
            .mapped(Self::row_from_sqlite)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list rows by method")?;

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        agents::{Agent, AgentKind},
        buffers::Buffer,
        rooms::Room,
    };

    fn setup() -> Result<(Database, String, String)> {
        let db = Database::in_memory()?;
        let room = Room::new("test");
        db.insert_room(&room)?;
        let buffer = Buffer::room_chat(&room.id);
        db.insert_buffer(&buffer)?;
        let agent = Agent::new("agent1", AgentKind::Human);
        db.insert_agent(&agent)?;
        Ok((db, buffer.id, agent.id))
    }

    #[test]
    fn test_row_crud() -> Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        let mut row = Row::message(&buffer_id, &agent_id, "Hello world", false);
        db.append_row(&mut row)?;

        let fetched = db.get_row(&row.id)?.expect("row should exist");
        assert_eq!(fetched.content, Some("Hello world".to_string()));
        assert_eq!(fetched.content_method, "message.user");
        assert!(!fetched.mutable);

        let rows = db.list_buffer_rows(&buffer_id)?;
        assert_eq!(rows.len(), 1);

        db.delete_row(&row.id)?;
        assert!(db.get_row(&row.id)?.is_none());

        Ok(())
    }

    #[test]
    fn test_row_ordering() -> Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        let mut row1 = Row::message(&buffer_id, &agent_id, "First", false);
        let mut row2 = Row::message(&buffer_id, &agent_id, "Second", false);
        let mut row3 = Row::message(&buffer_id, &agent_id, "Third", false);

        db.append_row(&mut row1)?;
        db.append_row(&mut row2)?;
        db.append_row(&mut row3)?;

        let rows = db.list_buffer_rows(&buffer_id)?;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].content, Some("First".to_string()));
        assert_eq!(rows[1].content, Some("Second".to_string()));
        assert_eq!(rows[2].content, Some("Third".to_string()));

        // Positions should be monotonically increasing
        assert!(rows[0].position < rows[1].position);
        assert!(rows[1].position < rows[2].position);

        Ok(())
    }

    #[test]
    fn test_row_nesting() -> Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        let mut parent = Row::message(&buffer_id, &agent_id, "Parent", false);
        db.append_row(&mut parent)?;

        let mut child1 = Row::message(&buffer_id, &agent_id, "Child 1", false);
        child1.parent_row_id = Some(parent.id.clone());
        child1.position = 0.0;
        db.insert_row(&child1)?;

        let mut child2 = Row::message(&buffer_id, &agent_id, "Child 2", false);
        child2.parent_row_id = Some(parent.id.clone());
        child2.position = 1.0;
        db.insert_row(&child2)?;

        // Top-level should only show parent
        let top_level = db.list_buffer_rows(&buffer_id)?;
        assert_eq!(top_level.len(), 1);

        // Children should be retrievable
        let children = db.list_child_rows(&parent.id)?;
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].content, Some("Child 1".to_string()));
        assert_eq!(children[1].content, Some("Child 2".to_string()));

        Ok(())
    }

    #[test]
    fn test_row_tags() -> Result<()> {
        let (db, buffer_id, _agent_id) = setup()?;

        let mut row = Row::new(&buffer_id, "note.user");
        row.content = Some("Important decision".to_string());
        db.append_row(&mut row)?;

        db.add_row_tag(&row.id, "#decision")?;
        db.add_row_tag(&row.id, "#important")?;

        let tags = db.get_row_tags(&row.id)?;
        assert_eq!(tags.len(), 2);
        assert!(tags.contains(&"#decision".to_string()));
        assert!(tags.contains(&"#important".to_string()));

        let found = db.find_rows_by_tag(&buffer_id, "#decision")?;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, row.id);

        db.remove_row_tag(&row.id, "#decision")?;
        let tags = db.get_row_tags(&row.id)?;
        assert_eq!(tags.len(), 1);

        Ok(())
    }

    #[test]
    fn test_row_reactions() -> Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        // Create additional agents for reactions
        let agent2 = Agent::new("agent2", AgentKind::Human);
        let agent3 = Agent::new("agent3", AgentKind::Human);
        db.insert_agent(&agent2)?;
        db.insert_agent(&agent3)?;

        let mut row = Row::message(&buffer_id, &agent_id, "Funny joke", false);
        db.append_row(&mut row)?;

        db.add_row_reaction(&row.id, &agent2.id, "laugh")?;
        db.add_row_reaction(&row.id, &agent3.id, "laugh")?;
        db.add_row_reaction(&row.id, &agent2.id, "clap")?;

        let reactions = db.get_row_reactions(&row.id)?;
        assert_eq!(reactions.len(), 3);

        db.remove_row_reaction(&row.id, &agent2.id, "laugh")?;
        let reactions = db.get_row_reactions(&row.id)?;
        assert_eq!(reactions.len(), 2);

        Ok(())
    }

    #[test]
    fn test_row_links() -> Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        // Create a second agent
        let agent2 = Agent::new("agent2", AgentKind::Model);
        db.insert_agent(&agent2)?;

        let mut row1 = Row::message(&buffer_id, &agent_id, "Original", false);
        let mut row2 = Row::message(&buffer_id, &agent2.id, "Reply", true);
        db.append_row(&mut row1)?;
        db.append_row(&mut row2)?;

        let link_id = db.create_row_link(&row2.id, &row1.id, LinkType::Reply)?;

        let from_links = db.get_row_links_from(&row2.id)?;
        assert_eq!(from_links.len(), 1);
        assert_eq!(from_links[0].link_type, LinkType::Reply);

        let to_links = db.get_row_links_to(&row1.id)?;
        assert_eq!(to_links.len(), 1);

        db.delete_row_link(&link_id)?;
        let from_links = db.get_row_links_from(&row2.id)?;
        assert!(from_links.is_empty());

        Ok(())
    }

    #[test]
    fn test_content_method_queries() -> Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        // Create a second agent for model messages
        let agent2 = Agent::new("model1", AgentKind::Model);
        db.insert_agent(&agent2)?;

        let mut msg1 = Row::message(&buffer_id, &agent_id, "User msg", false);
        let mut msg2 = Row::message(&buffer_id, &agent2.id, "Model msg", true);
        let mut sys = Row::system(&buffer_id, "System announcement");

        db.append_row(&mut msg1)?;
        db.append_row(&mut msg2)?;
        db.append_row(&mut sys)?;

        let all_messages = db.list_rows_by_method(&buffer_id, "message.%")?;
        assert_eq!(all_messages.len(), 3);

        let user_messages = db.list_rows_by_method(&buffer_id, "message.user")?;
        assert_eq!(user_messages.len(), 1);

        let model_messages = db.list_rows_by_method(&buffer_id, "message.model")?;
        assert_eq!(model_messages.len(), 1);

        Ok(())
    }

    #[test]
    fn test_fractional_indexing() {
        assert_eq!(fractional::midpoint(0.0, 1.0), 0.5);
        assert_eq!(fractional::after(5.0), 6.0);
        assert_eq!(fractional::before(5.0), 4.0);
        assert!(!fractional::needs_rebalance(0.0, 0.5));
        assert!(fractional::needs_rebalance(0.0, 1e-11));
    }

    #[test]
    fn test_append_to_row() -> Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        // Create a mutable row (like for streaming)
        let mut row = Row::thinking(&buffer_id, &agent_id);
        row.content = Some("Hello".to_string());
        db.append_row(&mut row)?;

        // Append more content
        db.append_to_row(&row.id, " world")?;
        db.append_to_row(&row.id, "!")?;

        let fetched = db.get_row(&row.id)?.expect("row should exist");
        assert_eq!(fetched.content, Some("Hello world!".to_string()));

        // Non-mutable rows should not be appended to
        let mut immutable = Row::message(&buffer_id, &agent_id, "Fixed", false);
        db.append_row(&mut immutable)?;
        db.append_to_row(&immutable.id, " extra")?;

        let fetched = db.get_row(&immutable.id)?.expect("row should exist");
        assert_eq!(fetched.content, Some("Fixed".to_string())); // Unchanged

        Ok(())
    }

    #[test]
    fn test_finalize_row() -> Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        // Create a mutable row
        let mut row = Row::thinking(&buffer_id, &agent_id);
        row.content = Some("Thinking...".to_string());
        db.append_row(&mut row)?;

        // Should be mutable before finalize
        let fetched = db.get_row(&row.id)?.expect("row should exist");
        assert!(fetched.mutable);
        assert!(fetched.finalized_at.is_none());

        // Finalize
        db.finalize_row(&row.id)?;

        let fetched = db.get_row(&row.id)?.expect("row should exist");
        assert!(!fetched.mutable);
        assert!(fetched.finalized_at.is_some());

        Ok(())
    }

    #[test]
    fn test_rows_since() -> Result<()> {
        let (db, buffer_id, agent_id) = setup()?;

        let mut row1 = Row::message(&buffer_id, &agent_id, "First", false);
        let mut row2 = Row::message(&buffer_id, &agent_id, "Second", false);
        let mut row3 = Row::message(&buffer_id, &agent_id, "Third", false);

        db.append_row(&mut row1)?;
        db.append_row(&mut row2)?;
        db.append_row(&mut row3)?;

        // Get rows since row1 (should be row2, row3)
        let since_1 = db.rows_since(&buffer_id, Some(&row1.id))?;
        assert_eq!(since_1.len(), 2);
        assert_eq!(since_1[0].content, Some("Second".to_string()));
        assert_eq!(since_1[1].content, Some("Third".to_string()));

        // Get rows since row2 (should be row3)
        let since_2 = db.rows_since(&buffer_id, Some(&row2.id))?;
        assert_eq!(since_2.len(), 1);
        assert_eq!(since_2[0].content, Some("Third".to_string()));

        // Get rows since row3 (should be empty)
        let since_3 = db.rows_since(&buffer_id, Some(&row3.id))?;
        assert!(since_3.is_empty());

        // Get all rows (since None)
        let all = db.rows_since(&buffer_id, None)?;
        assert_eq!(all.len(), 3);

        Ok(())
    }
}
