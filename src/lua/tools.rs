//! Tool bridge for Lua HUD
//!
//! Provides Lua functions that bridge to Rust state and MCP tools.
//! All functions are registered in a `tools` global table.

use crate::display::hud::HudState;
use crate::lua::cache::ToolCache;
use crate::lua::context::{build_hud_context, build_notifications_table, PendingNotification};
use crate::lua::mcp_bridge::McpBridge;
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

    // tools.cached(key) -> value or nil (alias: kv_get)
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
    tools.set("cached", cached_fn.clone())?;
    tools.set("kv_get", cached_fn)?;

    // tools.kv_set(key, value) -> nil
    let kv_set_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, (key, value): (String, Value)| {
            let json_value = lua_to_json(&value)?;
            state.cache.set_blocking(key, json_value);
            Ok(())
        })?
    };
    tools.set("kv_set", kv_set_fn)?;

    // tools.kv_delete(key) -> old value or nil
    let kv_delete_fn = {
        let state = state.clone();
        lua.create_function(move |lua, key: String| {
            if let Some(value) = state.cache.remove_blocking(&key) {
                json_to_lua(lua, &value)
            } else {
                Ok(Value::Nil)
            }
        })?
    };
    tools.set("kv_delete", kv_delete_fn)?;

    // Room/participant tools (mirrors sshwarma_* internal tools)

    // tools.look() -> room summary
    let look_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let hud = state.hud_state();
            let result = lua.create_table()?;

            // Room name
            if let Some(ref room) = hud.room_name {
                result.set("room", room.clone())?;
            } else {
                result.set("room", Value::Nil)?;
            }

            // Description
            if let Some(ref desc) = hud.description {
                result.set("description", desc.clone())?;
            } else {
                result.set("description", Value::Nil)?;
            }

            // Vibe
            if let Some(ref vibe) = hud.vibe {
                result.set("vibe", vibe.clone())?;
            } else {
                result.set("vibe", Value::Nil)?;
            }

            // Users array
            let users = lua.create_table()?;
            let mut user_idx = 1;
            for p in &hud.participants {
                if p.is_user() {
                    users.set(user_idx, p.name.clone())?;
                    user_idx += 1;
                }
            }
            result.set("users", users)?;

            // Models array
            let models = lua.create_table()?;
            let mut model_idx = 1;
            for p in &hud.participants {
                if p.is_model() {
                    models.set(model_idx, p.name.clone())?;
                    model_idx += 1;
                }
            }
            result.set("models", models)?;

            // Exits table
            let exits = lua.create_table()?;
            for (dir, room) in &hud.exits {
                exits.set(dir.clone(), room.clone())?;
            }
            result.set("exits", exits)?;

            Ok(result)
        })?
    };
    tools.set("look", look_fn)?;

    // tools.who() -> participant list
    let who_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let hud = state.hud_state();
            let list = lua.create_table()?;

            for (i, p) in hud.participants.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("name", p.name.clone())?;
                entry.set("is_model", p.is_model())?;
                entry.set("status", p.status.text())?;
                entry.set("glyph", p.status.glyph())?;
                list.set(i + 1, entry)?;
            }

            Ok(list)
        })?
    };
    tools.set("who", who_fn)?;

    // tools.exits() -> exit list
    let exits_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let hud = state.hud_state();
            let list = lua.create_table()?;

            for (i, (dir, dest)) in hud.exits.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("direction", dir.clone())?;
                entry.set("destination", dest.clone())?;
                list.set(i + 1, entry)?;
            }

            Ok(list)
        })?
    };
    tools.set("exits", exits_fn)?;

    // tools.vibe() -> string or nil
    let vibe_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, ()| {
            let hud = state.hud_state();
            Ok(hud.vibe.clone())
        })?
    };
    tools.set("vibe", vibe_fn)?;

    // tools.mcp_connections() -> MCP connections
    let mcp_connections_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let hud = state.hud_state();
            let list = lua.create_table()?;

            for (i, m) in hud.mcp_connections.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("name", m.name.clone())?;
                entry.set("tools", m.tool_count)?;
                entry.set("connected", m.connected)?;
                entry.set("calls", m.call_count)?;
                if let Some(ref last_tool) = m.last_tool {
                    entry.set("last_tool", last_tool.clone())?;
                }
                list.set(i + 1, entry)?;
            }

            Ok(list)
        })?
    };
    tools.set("mcp_connections", mcp_connections_fn)?;

    // tools.session() -> session info
    let session_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let hud = state.hud_state();
            let result = lua.create_table()?;
            result.set("start_ms", hud.session_start.timestamp_millis())?;
            result.set("duration", hud.duration_string())?;
            result.set("spinner_frame", hud.spinner_frame)?;
            Ok(result)
        })?
    };
    tools.set("session", session_fn)?;

    // Set as global
    lua.globals().set("tools", tools)?;

    Ok(())
}

/// Register MCP bridge functions in the Lua state
///
/// Adds to existing `tools` table:
/// - `tools.mcp_call(server, tool, args)` - Queue async MCP call, returns request_id
/// - `tools.mcp_result(request_id)` - Check result, returns (result, status)
pub fn register_mcp_tools(lua: &Lua, bridge: Arc<McpBridge>) -> LuaResult<()> {
    let tools: Table = lua.globals().get("tools")?;

    // tools.mcp_call(server, tool, args) -> request_id
    let mcp_call_fn = {
        let bridge = bridge.clone();
        lua.create_function(move |_lua, (server, tool, args): (String, String, Value)| {
            let json_args = lua_to_json(&args)?;
            let request_id = bridge.call(&server, &tool, json_args);
            Ok(request_id)
        })?
    };
    tools.set("mcp_call", mcp_call_fn)?;

    // tools.mcp_result(request_id) -> (result, status)
    let mcp_result_fn = {
        let bridge = bridge.clone();
        lua.create_function(move |lua, request_id: String| {
            let (result, status) = bridge.result(&request_id);
            let lua_result = match result {
                Some(v) => json_to_lua(lua, &v)?,
                None => Value::Nil,
            };
            Ok((lua_result, status))
        })?
    };
    tools.set("mcp_result", mcp_result_fn)?;

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

    #[test]
    fn test_kv_api() {
        let lua = Lua::new();
        let state = LuaToolState::new();

        register_tools(&lua, state).expect("should register tools");

        // Test kv_set and kv_get
        lua.load(r#"
            tools.kv_set("test.key", {foo = "bar", count = 42})
            local val = tools.kv_get("test.key")
            assert(val.foo == "bar", "foo should be bar")
            assert(val.count == 42, "count should be 42")
        "#)
        .exec()
        .expect("kv_set/kv_get should work");

        // Test kv_delete
        lua.load(r#"
            local old = tools.kv_delete("test.key")
            assert(old.foo == "bar", "deleted value should have foo")
            local gone = tools.kv_get("test.key")
            assert(gone == nil, "key should be deleted")
        "#)
        .exec()
        .expect("kv_delete should work");
    }
}
