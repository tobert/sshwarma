//! Lua script storage
//!
//! Reusable Lua code blocks for handlers, renderers, and transformers.

use super::{new_id, now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Script kind discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptKind {
    /// Event handler (row triggers, ticks)
    Handler,
    /// Region renderer
    Renderer,
    /// Content transformer
    Transformer,
}

impl ScriptKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScriptKind::Handler => "handler",
            ScriptKind::Renderer => "renderer",
            ScriptKind::Transformer => "transformer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "handler" => Some(ScriptKind::Handler),
            "renderer" => Some(ScriptKind::Renderer),
            "transformer" => Some(ScriptKind::Transformer),
            _ => None,
        }
    }
}

/// A Lua script in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaScript {
    pub id: String,
    pub name: Option<String>,
    pub kind: ScriptKind,
    pub code: String,
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl LuaScript {
    /// Create a new script
    pub fn new(name: impl Into<String>, kind: impl AsRef<str>, code: impl Into<String>) -> Self {
        let now = now_ms();
        Self {
            id: new_id(),
            name: Some(name.into()),
            kind: ScriptKind::parse(kind.as_ref()).unwrap_or(ScriptKind::Handler),
            code: code.into(),
            description: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create an anonymous script (no name)
    pub fn anonymous(kind: ScriptKind, code: impl Into<String>) -> Self {
        let now = now_ms();
        Self {
            id: new_id(),
            name: None,
            kind,
            code: code.into(),
            description: None,
            created_at: now,
            updated_at: now,
        }
    }
}

// Database operations
impl Database {
    /// Insert a new script
    pub fn insert_script(&self, script: &LuaScript) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO lua_scripts (id, name, script_kind, code, description, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                script.id,
                script.name,
                script.kind.as_str(),
                script.code,
                script.description,
                script.created_at,
                script.updated_at,
            ],
        )
        .context("failed to insert script")?;
        Ok(())
    }

    /// Get script by ID
    pub fn get_script(&self, id: &str) -> Result<Option<LuaScript>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, name, script_kind, code, description, created_at, updated_at
            FROM lua_scripts WHERE id = ?1
            "#,
            )
            .context("failed to prepare script query")?;

        let script = stmt
            .query_row(params![id], Self::script_from_row)
            .optional()
            .context("failed to query script")?;

        Ok(script)
    }

    /// Get script by name
    pub fn get_script_by_name(&self, name: &str) -> Result<Option<LuaScript>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, name, script_kind, code, description, created_at, updated_at
            FROM lua_scripts WHERE name = ?1
            "#,
            )
            .context("failed to prepare script query")?;

        let script = stmt
            .query_row(params![name], Self::script_from_row)
            .optional()
            .context("failed to query script by name")?;

        Ok(script)
    }

    /// List all scripts, optionally filtered by kind
    pub fn list_scripts(&self, kind: Option<ScriptKind>) -> Result<Vec<LuaScript>> {
        let conn = self.conn()?;
        let sql = match kind {
            Some(_) => {
                r#"
                SELECT id, name, script_kind, code, description, created_at, updated_at
                FROM lua_scripts WHERE script_kind = ?1 ORDER BY name
            "#
            }
            None => {
                r#"
                SELECT id, name, script_kind, code, description, created_at, updated_at
                FROM lua_scripts ORDER BY name
            "#
            }
        };

        let mut stmt = conn
            .prepare(sql)
            .context("failed to prepare scripts query")?;
        let rows = match kind {
            Some(k) => stmt.query(params![k.as_str()])?,
            None => stmt.query([])?,
        };

        let scripts = rows
            .mapped(Self::script_from_row)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list scripts")?;

        Ok(scripts)
    }

    /// Update a script (code and description)
    pub fn update_script(&self, script: &LuaScript) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE lua_scripts SET
                name = ?2, script_kind = ?3, code = ?4, description = ?5, updated_at = ?6
            WHERE id = ?1
            "#,
            params![
                script.id,
                script.name,
                script.kind.as_str(),
                script.code,
                script.description,
                now_ms(),
            ],
        )
        .context("failed to update script")?;
        Ok(())
    }

    /// Update just the code of a script
    pub fn update_script_code(&self, id: &str, code: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE lua_scripts SET code = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, code, now_ms()],
        )
        .context("failed to update script code")?;
        Ok(())
    }

    /// Delete a script
    pub fn delete_script(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM lua_scripts WHERE id = ?1", params![id])
            .context("failed to delete script")?;
        Ok(())
    }

    fn script_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LuaScript> {
        let kind_str: String = row.get(2)?;
        Ok(LuaScript {
            id: row.get(0)?,
            name: row.get(1)?,
            kind: ScriptKind::parse(&kind_str).unwrap_or(ScriptKind::Handler),
            code: row.get(3)?,
            description: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_crud() -> Result<()> {
        let db = Database::in_memory()?;

        let script = LuaScript::new(
            "default_hud",
            "renderer",
            r#"
            function render_hud(area, state)
                area:print(0, 0, "Hello HUD")
            end
            "#,
        );

        db.insert_script(&script)?;

        let fetched = db.get_script(&script.id)?.expect("script should exist");
        assert_eq!(fetched.name, Some("default_hud".to_string()));
        assert_eq!(fetched.kind, ScriptKind::Renderer);
        assert!(fetched.code.contains("render_hud"));

        let by_name = db
            .get_script_by_name("default_hud")?
            .expect("should find by name");
        assert_eq!(by_name.id, script.id);

        db.delete_script(&script.id)?;
        assert!(db.get_script(&script.id)?.is_none());

        Ok(())
    }

    #[test]
    fn test_script_kinds() -> Result<()> {
        let db = Database::in_memory()?;

        let handler = LuaScript::new("my_handler", "handler", "return true");
        let renderer = LuaScript::new("my_renderer", "renderer", "area:clear()");
        let transformer = LuaScript::new("my_transformer", "transformer", "return row");

        db.insert_script(&handler)?;
        db.insert_script(&renderer)?;
        db.insert_script(&transformer)?;

        let all = db.list_scripts(None)?;
        assert_eq!(all.len(), 3);

        let handlers = db.list_scripts(Some(ScriptKind::Handler))?;
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0].name, Some("my_handler".to_string()));

        let renderers = db.list_scripts(Some(ScriptKind::Renderer))?;
        assert_eq!(renderers.len(), 1);

        Ok(())
    }

    #[test]
    fn test_script_update() -> Result<()> {
        let db = Database::in_memory()?;

        let mut script = LuaScript::new("updatable", "handler", "-- version 1");
        db.insert_script(&script)?;

        // Update just code
        db.update_script_code(&script.id, "-- version 2")?;
        let fetched = db.get_script(&script.id)?.unwrap();
        assert!(fetched.code.contains("version 2"));
        // updated_at should be >= original (may be equal if within same millisecond)
        assert!(fetched.updated_at >= script.updated_at);

        // Update full script
        script.description = Some("A test script".to_string());
        script.code = "-- version 3".to_string();
        db.update_script(&script)?;

        let fetched = db.get_script(&script.id)?.unwrap();
        assert!(fetched.code.contains("version 3"));
        assert_eq!(fetched.description, Some("A test script".to_string()));

        Ok(())
    }

    #[test]
    fn test_anonymous_script() -> Result<()> {
        let db = Database::in_memory()?;

        let script = LuaScript::anonymous(ScriptKind::Handler, "return true");
        assert!(script.name.is_none());

        db.insert_script(&script)?;

        let fetched = db.get_script(&script.id)?.expect("script should exist");
        assert!(fetched.name.is_none());
        assert_eq!(fetched.kind, ScriptKind::Handler);

        // Can't find by name since it has none
        assert!(db.get_script_by_name("")?.is_none());

        Ok(())
    }
}
