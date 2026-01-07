//! Room rules CRUD operations
//!
//! Rules trigger Lua scripts based on row events or time intervals.

use super::{new_id, now_ms, Database};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Trigger kind discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    /// Fires when a matching row is created/updated
    Row,
    /// Fires every N milliseconds
    Interval,
    /// Fires every N background ticks (500ms base)
    Tick,
}

impl TriggerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TriggerKind::Row => "row",
            TriggerKind::Interval => "interval",
            TriggerKind::Tick => "tick",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "row" => Some(TriggerKind::Row),
            "interval" => Some(TriggerKind::Interval),
            "tick" => Some(TriggerKind::Tick),
            _ => None,
        }
    }
}

/// Action slot discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionSlot {
    /// Called during region render
    Render,
    /// Called during wrap() context building
    Wrap,
    /// Fires notification
    Notify,
    /// Can modify the row before display
    Transform,
    /// Runs in background loop
    Background,
}

impl ActionSlot {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActionSlot::Render => "render",
            ActionSlot::Wrap => "wrap",
            ActionSlot::Notify => "notify",
            ActionSlot::Transform => "transform",
            ActionSlot::Background => "background",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "render" => Some(ActionSlot::Render),
            "wrap" => Some(ActionSlot::Wrap),
            "notify" => Some(ActionSlot::Notify),
            "transform" => Some(ActionSlot::Transform),
            "background" => Some(ActionSlot::Background),
            _ => None,
        }
    }
}

/// A room rule in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomRule {
    pub id: String,
    pub room_id: String,
    pub name: Option<String>,
    pub enabled: bool,
    pub priority: f64,

    // Trigger kind
    pub trigger_kind: TriggerKind,

    // Row trigger conditions (all non-NULL must match)
    pub match_content_method: Option<String>, // glob: 'message.*', 'tool.call'
    pub match_source_agent: Option<String>,   // glob: 'claude', 'human:*', '*'
    pub match_tag: Option<String>,            // exact: '#decision'
    pub match_buffer_type: Option<String>,    // exact: 'room_chat', 'thinking'

    // Time trigger conditions
    pub interval_ms: Option<i64>,  // for 'interval': run every N ms
    pub tick_divisor: Option<i32>, // for 'tick': run every N ticks

    // Action
    pub script_id: String,
    pub action_slot: ActionSlot,

    pub created_at: i64,
}

impl RoomRule {
    /// Create a new row-triggered rule
    pub fn row_trigger(
        room_id: impl Into<String>,
        script_id: impl Into<String>,
        slot: ActionSlot,
    ) -> Self {
        Self {
            id: new_id(),
            room_id: room_id.into(),
            name: None,
            enabled: true,
            priority: 0.0,
            trigger_kind: TriggerKind::Row,
            match_content_method: None,
            match_source_agent: None,
            match_tag: None,
            match_buffer_type: None,
            interval_ms: None,
            tick_divisor: None,
            script_id: script_id.into(),
            action_slot: slot,
            created_at: now_ms(),
        }
    }

    /// Create a new tick-triggered rule
    pub fn tick_trigger(
        room_id: impl Into<String>,
        script_id: impl Into<String>,
        divisor: i32,
    ) -> Self {
        Self {
            id: new_id(),
            room_id: room_id.into(),
            name: None,
            enabled: true,
            priority: 0.0,
            trigger_kind: TriggerKind::Tick,
            match_content_method: None,
            match_source_agent: None,
            match_tag: None,
            match_buffer_type: None,
            interval_ms: None,
            tick_divisor: Some(divisor),
            script_id: script_id.into(),
            action_slot: ActionSlot::Background,
            created_at: now_ms(),
        }
    }

    /// Create a new interval-triggered rule
    pub fn interval_trigger(
        room_id: impl Into<String>,
        script_id: impl Into<String>,
        interval_ms: i64,
    ) -> Self {
        Self {
            id: new_id(),
            room_id: room_id.into(),
            name: None,
            enabled: true,
            priority: 0.0,
            trigger_kind: TriggerKind::Interval,
            match_content_method: None,
            match_source_agent: None,
            match_tag: None,
            match_buffer_type: None,
            interval_ms: Some(interval_ms),
            tick_divisor: None,
            script_id: script_id.into(),
            action_slot: ActionSlot::Background,
            created_at: now_ms(),
        }
    }
}

// Database operations
impl Database {
    /// Insert a new rule
    pub fn insert_rule(&self, rule: &RoomRule) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO room_rules (
                id, room_id, name, enabled, priority,
                trigger_kind, match_content_method, match_source_agent, match_tag, match_buffer_type,
                interval_ms, tick_divisor, script_id, action_slot, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            "#,
            params![
                rule.id,
                rule.room_id,
                rule.name,
                rule.enabled as i32,
                rule.priority,
                rule.trigger_kind.as_str(),
                rule.match_content_method,
                rule.match_source_agent,
                rule.match_tag,
                rule.match_buffer_type,
                rule.interval_ms,
                rule.tick_divisor,
                rule.script_id,
                rule.action_slot.as_str(),
                rule.created_at,
            ],
        )
        .context("failed to insert rule")?;
        Ok(())
    }

    /// Get rule by ID
    pub fn get_rule(&self, id: &str) -> Result<Option<RoomRule>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, room_id, name, enabled, priority,
                   trigger_kind, match_content_method, match_source_agent, match_tag, match_buffer_type,
                   interval_ms, tick_divisor, script_id, action_slot, created_at
            FROM room_rules WHERE id = ?1
            "#,
            )
            .context("failed to prepare rule query")?;

        let rule = stmt
            .query_row(params![id], Self::rule_from_row)
            .optional()
            .context("failed to query rule")?;

        Ok(rule)
    }

    /// List enabled rules for a room, ordered by priority
    pub fn list_room_rules(&self, room_id: &str) -> Result<Vec<RoomRule>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, room_id, name, enabled, priority,
                   trigger_kind, match_content_method, match_source_agent, match_tag, match_buffer_type,
                   interval_ms, tick_divisor, script_id, action_slot, created_at
            FROM room_rules
            WHERE room_id = ?1 AND enabled = 1
            ORDER BY priority
            "#,
            )
            .context("failed to prepare rules query")?;

        let rules = stmt
            .query(params![room_id])?
            .mapped(Self::rule_from_row)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list rules")?;

        Ok(rules)
    }

    /// List wrap rules for a specific target agent in a room
    pub fn list_wrap_rules_for_target(
        &self,
        room_id: &str,
        target_agent: &str,
    ) -> Result<Vec<RoomRule>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, room_id, name, enabled, priority,
                   trigger_kind, match_content_method, match_source_agent, match_tag, match_buffer_type,
                   interval_ms, tick_divisor, script_id, action_slot, created_at
            FROM room_rules
            WHERE room_id = ?1 AND enabled = 1 AND action_slot = 'wrap' AND match_source_agent = ?2
            ORDER BY priority
            "#,
            )
            .context("failed to prepare wrap rules query")?;

        let rules = stmt
            .query(params![room_id, target_agent])?
            .mapped(Self::rule_from_row)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list wrap rules")?;

        Ok(rules)
    }

    /// List all unique targets with wrap rules in a room
    pub fn list_targets_with_wrap_rules(&self, room_id: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT DISTINCT match_source_agent, name
            FROM room_rules
            WHERE room_id = ?1 AND enabled = 1 AND action_slot = 'wrap' AND match_source_agent IS NOT NULL
            ORDER BY match_source_agent
            "#,
            )
            .context("failed to prepare targets query")?;

        let mut results = Vec::new();
        let mut rows = stmt.query(params![room_id])?;
        let mut seen = std::collections::HashSet::new();

        while let Some(row) = rows.next()? {
            let target: String = row.get(0)?;
            let name: Option<String> = row.get(1)?;

            if seen.contains(&target) {
                continue;
            }
            seen.insert(target.clone());

            // Extract target_type from name (format: "target_type:prompt_name")
            let target_type = name
                .as_ref()
                .and_then(|n| n.split(':').next())
                .unwrap_or("unknown")
                .to_string();

            results.push((target, target_type));
        }

        Ok(results)
    }

    /// Get the highest priority value for a target's wrap rules
    pub fn max_wrap_priority(&self, room_id: &str, target_agent: &str) -> Result<Option<f64>> {
        let conn = self.conn()?;
        let priority: Option<f64> = conn
            .query_row(
                r#"
            SELECT MAX(priority)
            FROM room_rules
            WHERE room_id = ?1 AND action_slot = 'wrap' AND match_source_agent = ?2
            "#,
                params![room_id, target_agent],
                |row| row.get(0),
            )
            .optional()
            .context("failed to get max priority")?
            .flatten();

        Ok(priority)
    }

    /// Shift priorities up for rules at or above a given priority
    pub fn shift_wrap_priorities(
        &self,
        room_id: &str,
        target_agent: &str,
        from_priority: f64,
    ) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE room_rules
            SET priority = priority + 1
            WHERE room_id = ?1 AND action_slot = 'wrap' AND match_source_agent = ?2 AND priority >= ?3
            "#,
            params![room_id, target_agent, from_priority],
        )
        .context("failed to shift priorities")?;
        Ok(())
    }

    /// Delete wrap rule by priority (index)
    pub fn delete_wrap_rule_by_priority(
        &self,
        room_id: &str,
        target_agent: &str,
        priority: f64,
    ) -> Result<bool> {
        let conn = self.conn()?;
        let affected = conn.execute(
            r#"
            DELETE FROM room_rules
            WHERE room_id = ?1 AND action_slot = 'wrap' AND match_source_agent = ?2 AND priority = ?3
            "#,
            params![room_id, target_agent, priority],
        )
        .context("failed to delete wrap rule")?;
        Ok(affected > 0)
    }

    /// List rules by trigger kind
    pub fn list_rules_by_trigger(&self, room_id: &str, kind: TriggerKind) -> Result<Vec<RoomRule>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT id, room_id, name, enabled, priority,
                   trigger_kind, match_content_method, match_source_agent, match_tag, match_buffer_type,
                   interval_ms, tick_divisor, script_id, action_slot, created_at
            FROM room_rules
            WHERE room_id = ?1 AND enabled = 1 AND trigger_kind = ?2
            ORDER BY priority
            "#,
            )
            .context("failed to prepare rules query")?;

        let rules = stmt
            .query(params![room_id, kind.as_str()])?
            .mapped(Self::rule_from_row)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list rules by trigger")?;

        Ok(rules)
    }

    /// Update a rule
    pub fn update_rule(&self, rule: &RoomRule) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE room_rules SET
                room_id = ?2, name = ?3, enabled = ?4, priority = ?5,
                trigger_kind = ?6, match_content_method = ?7, match_source_agent = ?8,
                match_tag = ?9, match_buffer_type = ?10,
                interval_ms = ?11, tick_divisor = ?12, script_id = ?13, action_slot = ?14
            WHERE id = ?1
            "#,
            params![
                rule.id,
                rule.room_id,
                rule.name,
                rule.enabled as i32,
                rule.priority,
                rule.trigger_kind.as_str(),
                rule.match_content_method,
                rule.match_source_agent,
                rule.match_tag,
                rule.match_buffer_type,
                rule.interval_ms,
                rule.tick_divisor,
                rule.script_id,
                rule.action_slot.as_str(),
            ],
        )
        .context("failed to update rule")?;
        Ok(())
    }

    /// Delete a rule
    pub fn delete_rule(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM room_rules WHERE id = ?1", params![id])
            .context("failed to delete rule")?;
        Ok(())
    }

    /// Enable/disable a rule
    pub fn set_rule_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE room_rules SET enabled = ?2 WHERE id = ?1",
            params![id, enabled as i32],
        )
        .context("failed to set rule enabled")?;
        Ok(())
    }

    fn rule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoomRule> {
        let enabled_int: i32 = row.get(3)?;
        let trigger_kind_str: String = row.get(5)?;
        let action_slot_str: String = row.get(13)?;

        Ok(RoomRule {
            id: row.get(0)?,
            room_id: row.get(1)?,
            name: row.get(2)?,
            enabled: enabled_int != 0,
            priority: row.get(4)?,
            trigger_kind: TriggerKind::parse(&trigger_kind_str).unwrap_or(TriggerKind::Row),
            match_content_method: row.get(6)?,
            match_source_agent: row.get(7)?,
            match_tag: row.get(8)?,
            match_buffer_type: row.get(9)?,
            interval_ms: row.get(10)?,
            tick_divisor: row.get(11)?,
            script_id: row.get(12)?,
            action_slot: ActionSlot::parse(&action_slot_str).unwrap_or(ActionSlot::Background),
            created_at: row.get(14)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::rooms::Room;
    use crate::db::scripts::ScriptScope;

    fn setup() -> Result<(Database, String, String)> {
        let db = Database::in_memory()?;
        let room = Room::new("test");
        db.insert_room(&room)?;

        let script_id = db.create_script(
            ScriptScope::Room,
            Some("test"),
            "test_script",
            "return true",
            "test",
        )?;

        Ok((db, room.id, script_id))
    }

    #[test]
    fn test_rule_crud() -> Result<()> {
        let (db, room_id, script_id) = setup()?;

        let mut rule = RoomRule::row_trigger(&room_id, &script_id, ActionSlot::Render);
        rule.name = Some("Highlight decisions".to_string());
        rule.match_tag = Some("#decision".to_string());

        db.insert_rule(&rule)?;

        let fetched = db.get_rule(&rule.id)?.expect("rule should exist");
        assert_eq!(fetched.name, Some("Highlight decisions".to_string()));
        assert_eq!(fetched.trigger_kind, TriggerKind::Row);
        assert_eq!(fetched.match_tag, Some("#decision".to_string()));
        assert!(fetched.enabled);

        let rules = db.list_room_rules(&room_id)?;
        assert_eq!(rules.len(), 1);

        db.delete_rule(&rule.id)?;
        assert!(db.get_rule(&rule.id)?.is_none());

        Ok(())
    }

    #[test]
    fn test_rule_priority() -> Result<()> {
        let (db, room_id, script_id) = setup()?;

        let mut rule1 = RoomRule::row_trigger(&room_id, &script_id, ActionSlot::Render);
        rule1.priority = 10.0;
        rule1.name = Some("Low priority".to_string());

        let mut rule2 = RoomRule::row_trigger(&room_id, &script_id, ActionSlot::Render);
        rule2.priority = 1.0;
        rule2.name = Some("High priority".to_string());

        let mut rule3 = RoomRule::row_trigger(&room_id, &script_id, ActionSlot::Render);
        rule3.priority = 5.0;
        rule3.name = Some("Medium priority".to_string());

        db.insert_rule(&rule1)?;
        db.insert_rule(&rule2)?;
        db.insert_rule(&rule3)?;

        let rules = db.list_room_rules(&room_id)?;
        assert_eq!(rules[0].name, Some("High priority".to_string()));
        assert_eq!(rules[1].name, Some("Medium priority".to_string()));
        assert_eq!(rules[2].name, Some("Low priority".to_string()));

        Ok(())
    }

    #[test]
    fn test_rule_enable_disable() -> Result<()> {
        let (db, room_id, script_id) = setup()?;

        let rule = RoomRule::row_trigger(&room_id, &script_id, ActionSlot::Notify);
        db.insert_rule(&rule)?;

        // Initially enabled
        let rules = db.list_room_rules(&room_id)?;
        assert_eq!(rules.len(), 1);

        // Disable
        db.set_rule_enabled(&rule.id, false)?;
        let rules = db.list_room_rules(&room_id)?;
        assert!(rules.is_empty()); // Only lists enabled rules

        // Re-enable
        db.set_rule_enabled(&rule.id, true)?;
        let rules = db.list_room_rules(&room_id)?;
        assert_eq!(rules.len(), 1);

        Ok(())
    }

    #[test]
    fn test_trigger_kinds() -> Result<()> {
        let (db, room_id, script_id) = setup()?;

        let row_rule = RoomRule::row_trigger(&room_id, &script_id, ActionSlot::Render);
        let tick_rule = RoomRule::tick_trigger(&room_id, &script_id, 2);
        let interval_rule = RoomRule::interval_trigger(&room_id, &script_id, 1000);

        db.insert_rule(&row_rule)?;
        db.insert_rule(&tick_rule)?;
        db.insert_rule(&interval_rule)?;

        let all = db.list_room_rules(&room_id)?;
        assert_eq!(all.len(), 3);

        let row_rules = db.list_rules_by_trigger(&room_id, TriggerKind::Row)?;
        assert_eq!(row_rules.len(), 1);

        let tick_rules = db.list_rules_by_trigger(&room_id, TriggerKind::Tick)?;
        assert_eq!(tick_rules.len(), 1);
        assert_eq!(tick_rules[0].tick_divisor, Some(2));

        let interval_rules = db.list_rules_by_trigger(&room_id, TriggerKind::Interval)?;
        assert_eq!(interval_rules.len(), 1);
        assert_eq!(interval_rules[0].interval_ms, Some(1000));

        Ok(())
    }

    #[test]
    fn test_match_conditions() -> Result<()> {
        let (db, room_id, script_id) = setup()?;

        let mut rule = RoomRule::row_trigger(&room_id, &script_id, ActionSlot::Transform);
        rule.match_content_method = Some("message.*".to_string());
        rule.match_source_agent = Some("claude".to_string());
        rule.match_buffer_type = Some("room_chat".to_string());

        db.insert_rule(&rule)?;

        let fetched = db.get_rule(&rule.id)?.expect("rule should exist");
        assert_eq!(fetched.match_content_method, Some("message.*".to_string()));
        assert_eq!(fetched.match_source_agent, Some("claude".to_string()));
        assert_eq!(fetched.match_buffer_type, Some("room_chat".to_string()));

        Ok(())
    }
}
