//! Lua script storage with copy-on-write versioning
//!
//! User and room Lua modules stored in the database with version history.
//! Updates create new rows with parent_id linking to previous version.

use super::{new_id, now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Script scope - who owns this script
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptScope {
    /// System-level scripts (rarely used, for bootstrap)
    System,
    /// User-owned scripts
    User,
    /// Room-provided modules
    Room,
}

impl ScriptScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScriptScope::System => "system",
            ScriptScope::User => "user",
            ScriptScope::Room => "room",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(ScriptScope::System),
            "user" => Some(ScriptScope::User),
            "room" => Some(ScriptScope::Room),
            _ => None,
        }
    }
}

/// A Lua script in the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaScript {
    pub id: String,
    pub scope: ScriptScope,
    pub scope_id: Option<String>, // username or room_name
    pub module_path: String,      // "screen", "ui.status", etc.
    pub code: String,
    pub parent_id: Option<String>, // previous version (CoW)
    pub description: Option<String>,
    pub created_at: i64,
    pub created_by: Option<String>,
}

/// Script kind for room rules (legacy, still needed for room_rules table)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptKind {
    Handler,
    Renderer,
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

// Database operations for scripts
impl Database {
    /// Get the current (most recent) version of a script
    pub fn get_current_script(
        &self,
        scope: ScriptScope,
        scope_id: Option<&str>,
        module_path: &str,
    ) -> Result<Option<LuaScript>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, scope, scope_id, module_path, code, parent_id, description, created_at, created_by
                FROM lua_scripts
                WHERE scope = ?1 AND (scope_id = ?2 OR (scope_id IS NULL AND ?2 IS NULL)) AND module_path = ?3
                ORDER BY created_at DESC, id DESC
                LIMIT 1
                "#,
            )
            .context("failed to prepare script query")?;

        let script = stmt
            .query_row(
                params![scope.as_str(), scope_id, module_path],
                Self::script_from_row,
            )
            .optional()
            .context("failed to query script")?;

        Ok(script)
    }

    /// Get script by ID (any version)
    pub fn get_script(&self, id: &str) -> Result<Option<LuaScript>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, scope, scope_id, module_path, code, parent_id, description, created_at, created_by
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

    /// Create a new script (first version)
    pub fn create_script(
        &self,
        scope: ScriptScope,
        scope_id: Option<&str>,
        module_path: &str,
        code: &str,
        created_by: &str,
    ) -> Result<String> {
        let id = new_id();
        let now = now_ms();
        let conn = self.conn()?;

        conn.execute(
            r#"
            INSERT INTO lua_scripts (id, scope, scope_id, module_path, code, parent_id, description, created_at, created_by)
            VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, ?7)
            "#,
            params![id, scope.as_str(), scope_id, module_path, code, now, created_by],
        )
        .context("failed to create script")?;

        Ok(id)
    }

    /// Update a script (CoW - creates new version with parent_id)
    /// Returns the new version's ID
    pub fn update_script(&self, script_id: &str, code: &str, updated_by: &str) -> Result<String> {
        // Get the current script to copy metadata
        let current = self
            .get_script(script_id)?
            .context("script not found for update")?;

        let new_id = new_id();
        let now = now_ms();
        let conn = self.conn()?;

        conn.execute(
            r#"
            INSERT INTO lua_scripts (id, scope, scope_id, module_path, code, parent_id, description, created_at, created_by)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                new_id,
                current.scope.as_str(),
                current.scope_id,
                current.module_path,
                code,
                script_id, // parent_id points to old version
                current.description,
                now,
                updated_by,
            ],
        )
        .context("failed to update script (CoW)")?;

        Ok(new_id)
    }

    /// Update script by scope/module_path (convenience wrapper for CoW update)
    /// Returns the new version's ID
    pub fn update_script_by_path(
        &self,
        scope: ScriptScope,
        scope_id: Option<&str>,
        module_path: &str,
        code: &str,
        updated_by: &str,
    ) -> Result<String> {
        let current = self
            .get_current_script(scope, scope_id, module_path)?
            .context("script not found for update")?;

        self.update_script(&current.id, code, updated_by)
    }

    /// Set description on a script (creates new version)
    pub fn set_script_description(
        &self,
        script_id: &str,
        description: Option<&str>,
        updated_by: &str,
    ) -> Result<String> {
        let current = self
            .get_script(script_id)?
            .context("script not found for description update")?;

        let new_id = new_id();
        let now = now_ms();
        let conn = self.conn()?;

        conn.execute(
            r#"
            INSERT INTO lua_scripts (id, scope, scope_id, module_path, code, parent_id, description, created_at, created_by)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                new_id,
                current.scope.as_str(),
                current.scope_id,
                current.module_path,
                current.code,
                script_id,
                description,
                now,
                updated_by,
            ],
        )
        .context("failed to set script description")?;

        Ok(new_id)
    }

    /// List current versions of scripts for a scope
    pub fn list_scripts(
        &self,
        scope: ScriptScope,
        scope_id: Option<&str>,
    ) -> Result<Vec<LuaScript>> {
        let conn = self.conn()?;

        // Get the most recent version of each module_path using (created_at, id) for stable ordering
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, scope, scope_id, module_path, code, parent_id, description, created_at, created_by
                FROM lua_scripts s1
                WHERE scope = ?1 AND (scope_id = ?2 OR (scope_id IS NULL AND ?2 IS NULL))
                  AND (created_at, id) = (
                      SELECT MAX(created_at), MAX(id) FROM lua_scripts s2
                      WHERE s2.scope = s1.scope
                        AND (s2.scope_id = s1.scope_id OR (s2.scope_id IS NULL AND s1.scope_id IS NULL))
                        AND s2.module_path = s1.module_path
                        AND s2.created_at = (
                            SELECT MAX(created_at) FROM lua_scripts s3
                            WHERE s3.scope = s1.scope
                              AND (s3.scope_id = s1.scope_id OR (s3.scope_id IS NULL AND s1.scope_id IS NULL))
                              AND s3.module_path = s1.module_path
                        )
                  )
                ORDER BY module_path
                "#,
            )
            .context("failed to prepare scripts list query")?;

        let scripts = stmt
            .query_map(params![scope.as_str(), scope_id], Self::script_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list scripts")?;

        Ok(scripts)
    }

    /// List all versions of a specific module (for history)
    pub fn list_script_versions(
        &self,
        scope: ScriptScope,
        scope_id: Option<&str>,
        module_path: &str,
    ) -> Result<Vec<LuaScript>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, scope, scope_id, module_path, code, parent_id, description, created_at, created_by
                FROM lua_scripts
                WHERE scope = ?1 AND (scope_id = ?2 OR (scope_id IS NULL AND ?2 IS NULL)) AND module_path = ?3
                ORDER BY created_at DESC, id DESC
                "#,
            )
            .context("failed to prepare script versions query")?;

        let scripts = stmt
            .query_map(
                params![scope.as_str(), scope_id, module_path],
                Self::script_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list script versions")?;

        Ok(scripts)
    }

    /// Delete all versions of a script by module path
    pub fn delete_script(
        &self,
        scope: ScriptScope,
        scope_id: Option<&str>,
        module_path: &str,
    ) -> Result<usize> {
        let conn = self.conn()?;
        let deleted = conn
            .execute(
                r#"
                DELETE FROM lua_scripts
                WHERE scope = ?1 AND (scope_id = ?2 OR (scope_id IS NULL AND ?2 IS NULL)) AND module_path = ?3
                "#,
                params![scope.as_str(), scope_id, module_path],
            )
            .context("failed to delete script")?;

        Ok(deleted)
    }

    fn script_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LuaScript> {
        let scope_str: String = row.get(1)?;
        Ok(LuaScript {
            id: row.get(0)?,
            scope: ScriptScope::parse(&scope_str).unwrap_or(ScriptScope::User),
            scope_id: row.get(2)?,
            module_path: row.get(3)?,
            code: row.get(4)?,
            parent_id: row.get(5)?,
            description: row.get(6)?,
            created_at: row.get(7)?,
            created_by: row.get(8)?,
        })
    }
}

// User UI config operations
impl Database {
    /// Get user's UI entrypoint module
    pub fn get_user_entrypoint(&self, username: &str) -> Result<Option<String>> {
        let conn = self.conn()?;
        let entrypoint = conn
            .query_row(
                "SELECT entrypoint_module FROM user_ui_config WHERE username = ?1",
                params![username],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .context("failed to query user entrypoint")?
            .flatten();

        Ok(entrypoint)
    }

    /// Set user's UI entrypoint module (None = use embedded default)
    pub fn set_user_entrypoint(&self, username: &str, module_path: Option<&str>) -> Result<()> {
        let conn = self.conn()?;
        let now = now_ms();

        conn.execute(
            r#"
            INSERT INTO user_ui_config (username, entrypoint_module, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(username) DO UPDATE SET
                entrypoint_module = excluded.entrypoint_module,
                updated_at = excluded.updated_at
            "#,
            params![username, module_path, now],
        )
        .context("failed to set user entrypoint")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_create_and_get() -> Result<()> {
        let db = Database::in_memory()?;

        let id = db.create_script(
            ScriptScope::User,
            Some("alice"),
            "screen",
            r#"return { on_tick = function() end }"#,
            "alice",
        )?;

        let script = db.get_script(&id)?.expect("script should exist");
        assert_eq!(script.scope, ScriptScope::User);
        assert_eq!(script.scope_id, Some("alice".to_string()));
        assert_eq!(script.module_path, "screen");
        assert!(script.code.contains("on_tick"));
        assert!(script.parent_id.is_none());

        Ok(())
    }

    #[test]
    fn test_get_current_script() -> Result<()> {
        let db = Database::in_memory()?;

        db.create_script(ScriptScope::User, Some("alice"), "screen", "-- v1", "alice")?;

        let current = db
            .get_current_script(ScriptScope::User, Some("alice"), "screen")?
            .expect("should find script");
        assert!(current.code.contains("v1"));

        Ok(())
    }

    #[test]
    fn test_cow_update() -> Result<()> {
        let db = Database::in_memory()?;

        // Create v1
        let id1 = db.create_script(
            ScriptScope::User,
            Some("alice"),
            "screen",
            "-- version 1",
            "alice",
        )?;

        // Update to v2 (CoW)
        let id2 = db.update_script(&id1, "-- version 2", "alice")?;
        assert_ne!(id1, id2);

        // Check v2 has parent_id pointing to v1
        let v2 = db.get_script(&id2)?.expect("v2 should exist");
        assert_eq!(v2.parent_id, Some(id1.clone()));
        assert!(v2.code.contains("version 2"));

        // v1 still exists
        let v1 = db.get_script(&id1)?.expect("v1 should still exist");
        assert!(v1.code.contains("version 1"));

        // Current version is v2
        let current = db
            .get_current_script(ScriptScope::User, Some("alice"), "screen")?
            .expect("should find current");
        assert_eq!(current.id, id2);

        Ok(())
    }

    #[test]
    fn test_list_scripts() -> Result<()> {
        let db = Database::in_memory()?;

        db.create_script(ScriptScope::User, Some("alice"), "screen", "-- a", "alice")?;
        db.create_script(
            ScriptScope::User,
            Some("alice"),
            "ui.status",
            "-- b",
            "alice",
        )?;
        db.create_script(ScriptScope::User, Some("bob"), "screen", "-- c", "bob")?;

        let alice_scripts = db.list_scripts(ScriptScope::User, Some("alice"))?;
        assert_eq!(alice_scripts.len(), 2);

        let bob_scripts = db.list_scripts(ScriptScope::User, Some("bob"))?;
        assert_eq!(bob_scripts.len(), 1);

        Ok(())
    }

    #[test]
    fn test_list_script_versions() -> Result<()> {
        let db = Database::in_memory()?;

        let id1 = db.create_script(ScriptScope::User, Some("alice"), "screen", "-- v1", "alice")?;
        let _id2 = db.update_script(&id1, "-- v2", "alice")?;
        let _id3 =
            db.update_script_by_path(ScriptScope::User, Some("alice"), "screen", "-- v3", "alice")?;

        let versions = db.list_script_versions(ScriptScope::User, Some("alice"), "screen")?;
        assert_eq!(versions.len(), 3);
        // Most recent first
        assert!(versions[0].code.contains("v3"));
        assert!(versions[2].code.contains("v1"));

        Ok(())
    }

    #[test]
    fn test_delete_script() -> Result<()> {
        let db = Database::in_memory()?;

        let id = db.create_script(
            ScriptScope::User,
            Some("alice"),
            "screen",
            "-- code",
            "alice",
        )?;
        db.update_script(&id, "-- v2", "alice")?;

        // Should delete all versions
        let deleted = db.delete_script(ScriptScope::User, Some("alice"), "screen")?;
        assert_eq!(deleted, 2);

        assert!(db
            .get_current_script(ScriptScope::User, Some("alice"), "screen")?
            .is_none());

        Ok(())
    }

    #[test]
    fn test_room_scripts() -> Result<()> {
        let db = Database::in_memory()?;

        db.create_script(
            ScriptScope::Room,
            Some("workshop"),
            "tools",
            "return { sample = true }",
            "system",
        )?;

        let script = db
            .get_current_script(ScriptScope::Room, Some("workshop"), "tools")?
            .expect("should find room script");
        assert_eq!(script.scope, ScriptScope::Room);
        assert_eq!(script.scope_id, Some("workshop".to_string()));

        Ok(())
    }

    #[test]
    fn test_user_entrypoint() -> Result<()> {
        let db = Database::in_memory()?;

        // Initially no entrypoint
        assert!(db.get_user_entrypoint("alice")?.is_none());

        // Set entrypoint
        db.set_user_entrypoint("alice", Some("my_screen"))?;
        assert_eq!(
            db.get_user_entrypoint("alice")?,
            Some("my_screen".to_string())
        );

        // Update entrypoint
        db.set_user_entrypoint("alice", Some("custom_ui"))?;
        assert_eq!(
            db.get_user_entrypoint("alice")?,
            Some("custom_ui".to_string())
        );

        // Clear entrypoint (back to default)
        db.set_user_entrypoint("alice", None)?;
        assert!(db.get_user_entrypoint("alice")?.is_none());

        Ok(())
    }

    #[test]
    fn test_system_scope() -> Result<()> {
        let db = Database::in_memory()?;

        db.create_script(
            ScriptScope::System,
            None,
            "bootstrap",
            "-- system bootstrap",
            "system",
        )?;

        let script = db
            .get_current_script(ScriptScope::System, None, "bootstrap")?
            .expect("should find system script");
        assert_eq!(script.scope, ScriptScope::System);
        assert!(script.scope_id.is_none());

        Ok(())
    }
}
