//! Room rules engine
//!
//! Matches rows against rules and executes Lua scripts in the appropriate slots.

use crate::db::rows::Row;
use crate::db::rules::{ActionSlot, RoomRule, TriggerKind};
use crate::db::Database;
use anyhow::Result;
use mlua::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Glob-style pattern matcher
/// Supports: * (any chars), ? (single char)
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let mut p_chars = pattern.chars().peekable();
    let mut t_chars = text.chars().peekable();

    while let Some(p) = p_chars.next() {
        match p {
            '*' => {
                // Skip consecutive stars
                while p_chars.peek() == Some(&'*') {
                    p_chars.next();
                }

                // If star is at end, match rest
                if p_chars.peek().is_none() {
                    return true;
                }

                // Try matching rest of pattern at each position
                let remaining_pattern: String = p_chars.collect();
                while t_chars.peek().is_some() {
                    let remaining_text: String = t_chars.clone().collect();
                    if glob_match(&remaining_pattern, &remaining_text) {
                        return true;
                    }
                    t_chars.next();
                }

                // Try matching at empty string too
                return glob_match(&remaining_pattern, "");
            }
            '?' => {
                if t_chars.next().is_none() {
                    return false;
                }
            }
            c => {
                if t_chars.next() != Some(c) {
                    return false;
                }
            }
        }
    }

    // Pattern exhausted, text should be too
    t_chars.peek().is_none()
}

/// Result of matching a rule against a row
#[derive(Debug, Clone)]
pub struct RuleMatch {
    pub rule: RoomRule,
    pub matched_by: MatchReason,
}

/// Why a rule matched
#[derive(Debug, Clone)]
pub enum MatchReason {
    /// Matched by row content/source/tag
    RowTrigger,
    /// Matched by tick divisor
    TickTrigger(u64), // current tick
    /// Matched by interval timer
    IntervalTrigger,
}

/// Timer state for interval-based rules
#[derive(Debug)]
struct IntervalTimer {
    last_run: Instant,
    interval: Duration,
}

/// Cached rules for a room
#[derive(Debug, Default)]
struct RoomRulesCache {
    rules: Vec<RoomRule>,
    loaded_at: Option<Instant>,
}

/// Rules engine for matching and executing room rules
pub struct RulesEngine {
    /// Cached rules per room
    cache: RwLock<HashMap<String, RoomRulesCache>>,
    /// Interval timers per rule ID
    interval_timers: RwLock<HashMap<String, IntervalTimer>>,
    /// Current tick counter
    tick: RwLock<u64>,
    /// Cache TTL
    cache_ttl: Duration,
}

impl Default for RulesEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RulesEngine {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            interval_timers: RwLock::new(HashMap::new()),
            tick: RwLock::new(0),
            cache_ttl: Duration::from_secs(60),
        }
    }

    /// Load rules for a room (with caching)
    pub fn load_rules(&self, db: &Database, room_id: &str) -> Result<Vec<RoomRule>> {
        // Check cache
        {
            let cache = self.cache.read().unwrap();
            if let Some(entry) = cache.get(room_id) {
                if let Some(loaded_at) = entry.loaded_at {
                    if loaded_at.elapsed() < self.cache_ttl {
                        return Ok(entry.rules.clone());
                    }
                }
            }
        }

        // Load from DB
        let rules = db.list_room_rules(room_id)?;

        // Update cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(
                room_id.to_string(),
                RoomRulesCache {
                    rules: rules.clone(),
                    loaded_at: Some(Instant::now()),
                },
            );
        }

        Ok(rules)
    }

    /// Invalidate cache for a room
    pub fn invalidate_cache(&self, room_id: &str) {
        let mut cache = self.cache.write().unwrap();
        cache.remove(room_id);
    }

    /// Increment tick counter and return current value
    pub fn tick(&self) -> u64 {
        let mut tick = self.tick.write().unwrap();
        *tick += 1;
        *tick
    }

    /// Get current tick without incrementing
    pub fn current_tick(&self) -> u64 {
        *self.tick.read().unwrap()
    }

    /// Match a row against rules for its room
    pub fn match_row(&self, db: &Database, row: &Row) -> Result<Vec<RuleMatch>> {
        let rules = self.load_rules(db, &row.buffer_id)?;
        let mut matches = Vec::new();

        for rule in rules {
            if !rule.enabled {
                continue;
            }

            if rule.trigger_kind != TriggerKind::Row {
                continue;
            }

            if self.row_matches_rule(row, &rule) {
                matches.push(RuleMatch {
                    rule,
                    matched_by: MatchReason::RowTrigger,
                });
            }
        }

        // Sort by priority
        matches.sort_by(|a, b| a.rule.priority.partial_cmp(&b.rule.priority).unwrap());

        Ok(matches)
    }

    /// Check if a row matches a rule's conditions
    fn row_matches_rule(&self, row: &Row, rule: &RoomRule) -> bool {
        // Check content_method glob
        if let Some(pattern) = &rule.match_content_method {
            if !glob_match(pattern, &row.content_method) {
                return false;
            }
        }

        // Check source_agent glob
        if let Some(pattern) = &rule.match_source_agent {
            let source = row.source_agent_id.as_deref().unwrap_or("");
            if !glob_match(pattern, source) {
                return false;
            }
        }

        // Check buffer_type exact match
        // Note: We don't have buffer_type on Row, would need to join with buffer
        // For now, skip this check

        // Check tag exact match
        // Note: Tags are stored separately, would need to query
        // For now, skip this check

        true
    }

    /// Get tick-triggered rules that should run this tick
    pub fn match_tick(&self, db: &Database, room_id: &str) -> Result<Vec<RuleMatch>> {
        let rules = self.load_rules(db, room_id)?;
        let current_tick = self.current_tick();
        let mut matches = Vec::new();

        for rule in rules {
            if !rule.enabled {
                continue;
            }

            if rule.trigger_kind != TriggerKind::Tick {
                continue;
            }

            if let Some(divisor) = rule.tick_divisor {
                if divisor > 0 && current_tick.is_multiple_of(divisor as u64) {
                    matches.push(RuleMatch {
                        rule,
                        matched_by: MatchReason::TickTrigger(current_tick),
                    });
                }
            }
        }

        matches.sort_by(|a, b| a.rule.priority.partial_cmp(&b.rule.priority).unwrap());

        Ok(matches)
    }

    /// Get interval-triggered rules that should run now
    pub fn match_interval(&self, db: &Database, room_id: &str) -> Result<Vec<RuleMatch>> {
        let rules = self.load_rules(db, room_id)?;
        let now = Instant::now();
        let mut matches = Vec::new();

        let mut timers = self.interval_timers.write().unwrap();

        for rule in rules {
            if !rule.enabled {
                continue;
            }

            if rule.trigger_kind != TriggerKind::Interval {
                continue;
            }

            if let Some(interval_ms) = rule.interval_ms {
                let interval = Duration::from_millis(interval_ms as u64);

                let should_run = match timers.get(&rule.id) {
                    Some(timer) => now.duration_since(timer.last_run) >= timer.interval,
                    None => true, // First run
                };

                if should_run {
                    timers.insert(
                        rule.id.clone(),
                        IntervalTimer {
                            last_run: now,
                            interval,
                        },
                    );

                    matches.push(RuleMatch {
                        rule,
                        matched_by: MatchReason::IntervalTrigger,
                    });
                }
            }
        }

        matches.sort_by(|a, b| a.rule.priority.partial_cmp(&b.rule.priority).unwrap());

        Ok(matches)
    }

    /// Get all matches for a specific action slot
    pub fn matches_for_slot<'a>(
        &self,
        matches: &'a [RuleMatch],
        slot: ActionSlot,
    ) -> Vec<&'a RuleMatch> {
        matches
            .iter()
            .filter(|m| m.rule.action_slot == slot)
            .collect()
    }
}

/// Lua userdata wrapper for rules engine
#[derive(Clone)]
pub struct LuaRulesEngine {
    engine: Arc<RulesEngine>,
    db: Arc<Database>,
}

impl LuaRulesEngine {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            engine: Arc::new(RulesEngine::new()),
            db,
        }
    }

    pub fn engine(&self) -> &RulesEngine {
        &self.engine
    }
}

impl LuaUserData for LuaRulesEngine {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        // rules:tick() -> current_tick
        methods.add_method("tick", |_lua, this, ()| Ok(this.engine.tick() as i64));

        // rules:current_tick() -> tick
        methods.add_method("current_tick", |_lua, this, ()| {
            Ok(this.engine.current_tick() as i64)
        });

        // rules:invalidate(room_id)
        methods.add_method("invalidate", |_lua, this, room_id: String| {
            this.engine.invalidate_cache(&room_id);
            Ok(())
        });

        // rules:match_tick(room_id) -> [{rule_id, script_id, slot}, ...]
        methods.add_method("match_tick", |lua, this, room_id: String| {
            let matches = this
                .engine
                .match_tick(&this.db, &room_id)
                .map_err(mlua::Error::external)?;

            let result = lua.create_table()?;
            for (i, m) in matches.iter().enumerate() {
                let tbl = lua.create_table()?;
                tbl.set("rule_id", m.rule.id.as_str())?;
                tbl.set("script_id", m.rule.script_id.as_str())?;
                tbl.set("slot", m.rule.action_slot.as_str())?;
                if let Some(name) = &m.rule.name {
                    tbl.set("name", name.as_str())?;
                }
                result.set(i + 1, tbl)?;
            }

            Ok(result)
        });

        // rules:match_interval(room_id) -> [{rule_id, script_id, slot}, ...]
        methods.add_method("match_interval", |lua, this, room_id: String| {
            let matches = this
                .engine
                .match_interval(&this.db, &room_id)
                .map_err(mlua::Error::external)?;

            let result = lua.create_table()?;
            for (i, m) in matches.iter().enumerate() {
                let tbl = lua.create_table()?;
                tbl.set("rule_id", m.rule.id.as_str())?;
                tbl.set("script_id", m.rule.script_id.as_str())?;
                tbl.set("slot", m.rule.action_slot.as_str())?;
                if let Some(name) = &m.rule.name {
                    tbl.set("name", name.as_str())?;
                }
                result.set(i + 1, tbl)?;
            }

            Ok(result)
        });
    }
}

/// Register rules functions in Lua
pub fn register_rules_functions(lua: &Lua, db: Arc<Database>) -> LuaResult<()> {
    let globals = lua.globals();
    let sshwarma: LuaTable = globals.get("sshwarma")?;

    let engine = LuaRulesEngine::new(db);
    sshwarma.set("rules", engine)?;

    // sshwarma.glob_match(pattern, text) -> bool
    sshwarma.set(
        "glob_match",
        lua.create_function(|_lua, (pattern, text): (String, String)| {
            Ok(glob_match(&pattern, &text))
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("hello", "hello"));
        assert!(!glob_match("hello", "world"));
        assert!(!glob_match("hello", "hello world"));
    }

    #[test]
    fn test_glob_match_star() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
        assert!(glob_match("hello*", "hello"));
        assert!(glob_match("hello*", "hello world"));
        assert!(glob_match("*world", "hello world"));
        assert!(glob_match("*world", "world"));
        assert!(glob_match("hello*world", "hello beautiful world"));
        assert!(!glob_match("hello*world", "hello beautiful"));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(glob_match("h?llo", "hello"));
        assert!(glob_match("h?llo", "hallo"));
        assert!(!glob_match("h?llo", "hllo"));
        assert!(!glob_match("h?llo", "heello"));
    }

    #[test]
    fn test_glob_match_combined() {
        assert!(glob_match("message.*", "message.user"));
        assert!(glob_match("message.*", "message.model"));
        assert!(!glob_match("message.*", "tool.call"));

        assert!(glob_match("human:*", "human:alice"));
        assert!(glob_match("human:*", "human:"));
        assert!(!glob_match("human:*", "model:claude"));

        assert!(glob_match("*.test.*", "foo.test.bar"));
        assert!(glob_match("*.test.*", "a.test.b"));
    }

    #[test]
    fn test_rules_engine_tick() {
        let engine = RulesEngine::new();

        assert_eq!(engine.current_tick(), 0);
        assert_eq!(engine.tick(), 1);
        assert_eq!(engine.tick(), 2);
        assert_eq!(engine.current_tick(), 2);
    }

    #[test]
    fn test_match_reason() {
        let reason = MatchReason::TickTrigger(42);
        if let MatchReason::TickTrigger(tick) = reason {
            assert_eq!(tick, 42);
        } else {
            panic!("Expected TickTrigger");
        }
    }

    #[test]
    fn test_lua_glob_match() -> anyhow::Result<()> {
        let lua = Lua::new();

        let sshwarma = lua.create_table()?;
        lua.globals().set("sshwarma", sshwarma)?;

        // Register just the glob_match function for testing
        let sshwarma: LuaTable = lua.globals().get("sshwarma")?;
        sshwarma.set(
            "glob_match",
            lua.create_function(|_lua, (pattern, text): (String, String)| {
                Ok(glob_match(&pattern, &text))
            })?,
        )?;

        lua.load(
            r#"
            assert(sshwarma.glob_match("message.*", "message.user"))
            assert(sshwarma.glob_match("*", "anything"))
            assert(not sshwarma.glob_match("message.*", "tool.call"))
            assert(sshwarma.glob_match("human:*", "human:alice"))
        "#,
        )
        .exec()?;

        Ok(())
    }
}
