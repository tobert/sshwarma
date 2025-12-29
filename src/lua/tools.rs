//! Tool bridge for Lua HUD
//!
//! Provides Lua functions that bridge to Rust state and MCP tools.
//! All functions are registered in a `tools` global table.

use crate::display::hud::HudState;
use crate::lua::cache::ToolCache;
use crate::lua::context::{build_hud_context, build_notifications_table, PendingNotification};
use mlua::{Lua, Result as LuaResult, Table, UserData, UserDataMethods, Value};
use std::sync::{Arc, RwLock};

/// Shared state holder for Lua callbacks
///
/// Uses Arc<RwLock<...>> for thread-safe interior mutability,
/// allowing the state to be shared across async handlers and
/// spawned tasks (required for russh's Send+Sync handler bounds).
#[derive(Clone)]
pub struct LuaToolState {
    /// Current HUD state (updated by Rust before each render)
    hud_state: Arc<RwLock<HudState>>,
    /// Pending notifications queue (Rust adds, Lua drains)
    pending_notifications: Arc<RwLock<Vec<PendingNotification>>>,
    /// Tool result cache for instant reads
    cache: ToolCache,
}

impl LuaToolState {
    /// Create a new tool state with default HUD state
    pub fn new() -> Self {
        Self {
            hud_state: Arc::new(RwLock::new(HudState::new())),
            pending_notifications: Arc::new(RwLock::new(Vec::new())),
            cache: ToolCache::new(),
        }
    }

    /// Update the HUD state (called before render)
    pub fn update_hud_state(&self, state: HudState) {
        if let Ok(mut guard) = self.hud_state.write() {
            *guard = state;
        }
    }

    /// Get a clone of the current HUD state
    pub fn hud_state(&self) -> HudState {
        self.hud_state.read().map(|g| g.clone()).unwrap_or_default()
    }

    /// Push a notification to the queue
    pub fn push_notification(&self, message: String, ttl_ms: i64) {
        let notification = PendingNotification {
            message,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            ttl_ms,
        };
        if let Ok(mut guard) = self.pending_notifications.write() {
            guard.push(notification);
        }
    }

    /// Get the cache for background updates
    pub fn cache(&self) -> &ToolCache {
        &self.cache
    }

    /// Drain all pending notifications
    fn drain_notifications(&self) -> Vec<PendingNotification> {
        self.pending_notifications
            .write()
            .map(|mut guard| std::mem::take(&mut *guard))
            .unwrap_or_default()
    }
}

impl Default for LuaToolState {
    fn default() -> Self {
        Self::new()
    }
}

/// Register all tool functions in the Lua state
///
/// Creates a global `tools` table with:
/// - `tools.hud_state()` - returns current HUD state as table
/// - `tools.clear_notifications()` - drains pending notifications
/// - `tools.cached(key)` - reads from cache
///
/// MCP tools can be registered dynamically via `register_mcp_tool`.
pub fn register_tools(lua: &Lua, state: LuaToolState) -> LuaResult<()> {
    let tools = lua.create_table()?;

    // Store state in Lua registry for access from callbacks
    lua.set_named_registry_value("tool_state", LuaToolStateWrapper(state.clone()))?;

    // tools.hud_state() -> table
    let hud_state_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let hud = state.hud_state();
            build_hud_context(lua, &hud)
        })?
    };
    tools.set("hud_state", hud_state_fn)?;

    // tools.clear_notifications() -> array of notifications
    let clear_notifications_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let notifications = state.drain_notifications();
            if notifications.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::Table(build_notifications_table(lua, &notifications)?))
            }
        })?
    };
    tools.set("clear_notifications", clear_notifications_fn)?;

    // tools.cached(key) -> value or nil
    let cached_fn = {
        let state = state.clone();
        lua.create_function(move |lua, key: String| {
            if let Some(value) = state.cache.get_data_blocking(&key) {
                // Convert serde_json::Value to Lua Value
                json_to_lua(lua, &value)
            } else {
                Ok(Value::Nil)
            }
        })?
    };
    tools.set("cached", cached_fn)?;

    // Set as global
    lua.globals().set("tools", tools)?;

    Ok(())
}

/// Wrapper to make LuaToolState work with Lua registry
#[allow(dead_code)]
struct LuaToolStateWrapper(LuaToolState);

impl UserData for LuaToolStateWrapper {
    fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {
        // We don't expose methods directly; access via registry
    }
}

/// Register an MCP tool as a Lua function
///
/// The tool function takes a table of arguments and returns
/// the result or nil + error message.
#[allow(dead_code)]
pub fn register_mcp_tool(
    lua: &Lua,
    name: &str,
    _description: &str,
    call_fn: impl Fn(serde_json::Value) -> Result<serde_json::Value, String> + Send + 'static,
) -> LuaResult<()> {
    let tools: Table = lua.globals().get("tools")?;

    let tool_fn = lua.create_function(move |lua, args: Value| {
        // Convert Lua args to JSON
        let json_args = lua_to_json(&args)?;

        // Call the tool
        match call_fn(json_args) {
            Ok(result) => json_to_lua(lua, &result),
            Err(_err) => {
                // Return nil for errors
                Ok(Value::Nil)
            }
        }
    })?;

    tools.set(name, tool_fn)?;

    Ok(())
}

/// Convert serde_json::Value to mlua::Value
fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> LuaResult<Value> {
    match value {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                Ok(Value::Number(f))
            } else {
                Ok(Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_json::Value::Object(obj) => {
            let table = lua.create_table()?;
            for (k, v) in obj {
                table.set(k.clone(), json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
    }
}

/// Convert mlua::Value to serde_json::Value
fn lua_to_json(value: &Value) -> LuaResult<serde_json::Value> {
    match value {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(i) => Ok(serde_json::Value::Number((*i).into())),
        Value::Number(n) => Ok(serde_json::json!(*n)),
        Value::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_string())),
        Value::Table(t) => {
            // Check if it's an array (sequential integer keys starting at 1)
            let mut is_array = true;
            let mut max_key = 0;
            for pair in t.clone().pairs::<Value, Value>() {
                let (k, _) = pair?;
                match k {
                    Value::Integer(i) if i > 0 => {
                        max_key = max_key.max(i as usize);
                    }
                    _ => {
                        is_array = false;
                        break;
                    }
                }
            }

            if is_array && max_key > 0 {
                let mut arr = Vec::with_capacity(max_key);
                for i in 1..=max_key {
                    let v: Value = t.get(i)?;
                    arr.push(lua_to_json(&v)?);
                }
                Ok(serde_json::Value::Array(arr))
            } else {
                let mut obj = serde_json::Map::new();
                for pair in t.clone().pairs::<String, Value>() {
                    let (k, v) = pair?;
                    obj.insert(k, lua_to_json(&v)?);
                }
                Ok(serde_json::Value::Object(obj))
            }
        }
        _ => Ok(serde_json::Value::Null), // Functions, userdata, etc. become null
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Function;

    #[test]
    fn test_register_tools() {
        let lua = Lua::new();
        let state = LuaToolState::new();

        register_tools(&lua, state).expect("should register tools");

        // Verify tools table exists
        let tools: Table = lua.globals().get("tools").expect("should have tools");

        // Verify functions exist
        let _hud_state: Function = tools.get("hud_state").expect("should have hud_state");
        let _clear: Function = tools
            .get("clear_notifications")
            .expect("should have clear_notifications");
        let _cached: Function = tools.get("cached").expect("should have cached");
    }

    #[test]
    fn test_json_to_lua_roundtrip() {
        let lua = Lua::new();

        let json = serde_json::json!({
            "name": "test",
            "count": 42,
            "active": true,
            "items": [1, 2, 3],
            "nested": {"foo": "bar"}
        });

        let lua_val = json_to_lua(&lua, &json).expect("should convert to lua");
        let back = lua_to_json(&lua_val).expect("should convert back");

        assert_eq!(json, back);
    }

    #[test]
    fn test_notification_queue() {
        let state = LuaToolState::new();

        state.push_notification("Hello".to_string(), 5000);
        state.push_notification("World".to_string(), 3000);

        let notifications = state.drain_notifications();
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[0].message, "Hello");
        assert_eq!(notifications[1].message, "World");

        // Queue should be empty now
        let empty = state.drain_notifications();
        assert!(empty.is_empty());
    }
}
