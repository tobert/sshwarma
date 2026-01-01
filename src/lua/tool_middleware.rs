//! Lua middleware for MCP tool routing, filtering, and transformation
//!
//! Provides hooks for Lua scripts to intercept and modify MCP tools:
//! - `on_mcp_tools(mcp_name, tools, context)` - filter/transform tools on refresh
//! - `on_tool_call(mcp_name, tool_name, args)` - intercept before execution
//! - `on_tool_result(mcp_name, tool_name, result, is_error)` - transform results
//!
//! Also provides routing configuration:
//! - `tools.set_tool_priority({tool = "mcp"})` - prefer specific MCP for tools
//! - `tools.alias_tool(alias, "mcp:tool")` - create tool aliases

use crate::mcp::ToolInfo;
use anyhow::Result;
use mlua::{Function, Lua, Table, Value};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tracing::{debug, warn};

/// Tool information with optional schema (for middleware processing)
#[derive(Debug, Clone)]
pub struct ToolInfoExt {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// Which MCP server provides this
    pub source: String,
    /// Input schema (optional, for schema transformations)
    pub input_schema: Option<serde_json::Value>,
}

/// Default timeout for Lua hook execution (100ms)
const HOOK_TIMEOUT_MS: u64 = 100;

/// Tool middleware layer for Lua-based routing and transformation
///
/// Thread-safe: all state in `Arc<RwLock<T>>`
#[derive(Clone)]
pub struct ToolMiddleware {
    /// Priority routing: tool_name → preferred_mcp
    priorities: Arc<RwLock<HashMap<String, String>>>,
    /// Aliases: alias → "mcp:tool"
    aliases: Arc<RwLock<HashMap<String, String>>>,
}

impl ToolMiddleware {
    /// Create a new empty middleware layer
    pub fn new() -> Self {
        Self {
            priorities: Arc::new(RwLock::new(HashMap::new())),
            aliases: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set priority routing for a tool
    ///
    /// When multiple MCPs provide the same tool, prefer the specified MCP.
    pub fn set_priority(&self, tool_name: &str, mcp_name: &str) {
        if let Ok(mut priorities) = self.priorities.write() {
            priorities.insert(tool_name.to_string(), mcp_name.to_string());
        }
    }

    /// Set multiple priorities at once
    pub fn set_priorities(&self, map: HashMap<String, String>) {
        if let Ok(mut priorities) = self.priorities.write() {
            *priorities = map;
        }
    }

    /// Create an alias for a tool
    ///
    /// The target should be in "mcp:tool" format.
    pub fn set_alias(&self, alias: &str, target: &str) {
        if let Ok(mut aliases) = self.aliases.write() {
            aliases.insert(alias.to_string(), target.to_string());
        }
    }

    /// Clear all priorities and aliases
    pub fn clear(&self) {
        if let Ok(mut priorities) = self.priorities.write() {
            priorities.clear();
        }
        if let Ok(mut aliases) = self.aliases.write() {
            aliases.clear();
        }
    }

    /// Resolve tool routing
    ///
    /// Returns `Some((mcp_name, tool_name))` if routing is configured,
    /// `None` to use default first-match behavior.
    pub fn resolve_tool(&self, name: &str) -> Option<(String, String)> {
        // 1. Check aliases first
        if let Ok(aliases) = self.aliases.read() {
            if let Some(target) = aliases.get(name) {
                let parts: Vec<&str> = target.split(':').collect();
                if parts.len() == 2 {
                    return Some((parts[0].to_string(), parts[1].to_string()));
                }
            }
        }

        // 2. Check priorities
        if let Ok(priorities) = self.priorities.read() {
            if let Some(preferred_mcp) = priorities.get(name) {
                return Some((preferred_mcp.clone(), name.to_string()));
            }
        }

        // 3. No routing configured, use default behavior
        None
    }

    /// Check if a tool has priority routing configured
    pub fn has_priority(&self, tool_name: &str) -> bool {
        self.priorities
            .read()
            .map(|p| p.contains_key(tool_name))
            .unwrap_or(false)
    }

    /// Get all configured priorities
    pub fn priorities(&self) -> HashMap<String, String> {
        self.priorities
            .read()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    /// Get all configured aliases
    pub fn aliases(&self) -> HashMap<String, String> {
        self.aliases
            .read()
            .map(|a| a.clone())
            .unwrap_or_default()
    }

    /// Call on_mcp_tools hook if defined
    ///
    /// Returns modified tools list, or original if hook not defined or fails.
    pub fn process_tools_refresh(
        &self,
        lua: &Lua,
        mcp_name: &str,
        tools: Vec<ToolInfo>,
        context: Option<&ToolContext>,
    ) -> Vec<ToolInfo> {
        // Convert to extended format for Lua processing
        let tools_ext: Vec<ToolInfoExt> = tools
            .iter()
            .map(|t| ToolInfoExt {
                name: t.name.clone(),
                description: t.description.clone(),
                source: t.source.clone(),
                input_schema: None, // Schema not available at this level
            })
            .collect();

        match self.call_on_mcp_tools(lua, mcp_name, &tools_ext, context) {
            Ok(Some(modified)) => {
                // Convert back to ToolInfo
                modified
                    .into_iter()
                    .map(|t| ToolInfo {
                        name: t.name,
                        description: t.description,
                        source: t.source,
                    })
                    .collect()
            }
            Ok(None) => tools, // Hook returned nil, keep original
            Err(e) => {
                warn!("on_mcp_tools hook failed: {}, keeping original tools", e);
                tools
            }
        }
    }

    /// Internal: Call the on_mcp_tools hook
    fn call_on_mcp_tools(
        &self,
        lua: &Lua,
        mcp_name: &str,
        tools: &[ToolInfoExt],
        context: Option<&ToolContext>,
    ) -> Result<Option<Vec<ToolInfoExt>>> {
        let globals = lua.globals();

        // Check if hook exists
        let hook: Value = globals.get("on_mcp_tools")?;
        if hook == Value::Nil {
            return Ok(None);
        }

        let func: Function = hook
            .as_function()
            .ok_or_else(|| anyhow::anyhow!("on_mcp_tools is not a function"))?
            .clone();

        // Convert tools to Lua table
        let tools_table = tools_to_lua(lua, tools)?;

        // Build context table
        let context_table = lua.create_table()?;
        if let Some(ctx) = context {
            if let Some(ref model) = ctx.target_model {
                context_table.set("target_model", model.as_str())?;
            }
            if let Some(ref backend) = ctx.model_backend {
                context_table.set("model_backend", backend.as_str())?;
            }
        }

        // Call hook with timeout
        let result: Value = call_with_timeout(
            || func.call((mcp_name.to_string(), tools_table, context_table)),
            Duration::from_millis(HOOK_TIMEOUT_MS),
        )?;

        // Parse result
        match result {
            Value::Nil => Ok(None),
            Value::Table(table) => {
                let modified = lua_to_tools(&table)?;
                Ok(Some(modified))
            }
            _ => Err(anyhow::anyhow!(
                "on_mcp_tools must return table or nil, got {:?}",
                result.type_name()
            )),
        }
    }

    /// Call on_tool_call hook before tool execution
    ///
    /// Returns `Some(modified_args)` to proceed, `None` to block the call.
    pub fn process_tool_call(
        &self,
        lua: &Lua,
        mcp_name: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<Option<serde_json::Value>> {
        let globals = lua.globals();

        // Check if hook exists
        let hook: Value = globals.get("on_tool_call")?;
        if hook == Value::Nil {
            return Ok(Some(args)); // No hook, proceed with original args
        }

        let func: Function = hook
            .as_function()
            .ok_or_else(|| anyhow::anyhow!("on_tool_call is not a function"))?
            .clone();

        // Convert args to Lua
        let args_table = json_to_lua(lua, &args)?;

        // Call hook
        let result: Value = call_with_timeout(
            || {
                func.call((
                    mcp_name.to_string(),
                    tool_name.to_string(),
                    args_table,
                ))
            },
            Duration::from_millis(HOOK_TIMEOUT_MS),
        )?;

        // Parse result
        match result {
            Value::Nil => {
                debug!("on_tool_call blocked {}:{}", mcp_name, tool_name);
                Ok(None)
            }
            Value::Table(table) => {
                let modified = lua_to_json(&Value::Table(table))?;
                Ok(Some(modified))
            }
            _ => Err(anyhow::anyhow!(
                "on_tool_call must return table or nil, got {:?}",
                result.type_name()
            )),
        }
    }

    /// Call on_tool_result hook after tool execution
    ///
    /// Returns modified result string.
    pub fn process_tool_result(
        &self,
        lua: &Lua,
        mcp_name: &str,
        tool_name: &str,
        result: &str,
        is_error: bool,
    ) -> String {
        match self.call_on_tool_result(lua, mcp_name, tool_name, result, is_error) {
            Ok(modified) => modified,
            Err(e) => {
                warn!("on_tool_result hook failed: {}, keeping original", e);
                result.to_string()
            }
        }
    }

    /// Internal: Call the on_tool_result hook
    fn call_on_tool_result(
        &self,
        lua: &Lua,
        mcp_name: &str,
        tool_name: &str,
        result: &str,
        is_error: bool,
    ) -> Result<String> {
        let globals = lua.globals();

        // Check if hook exists
        let hook: Value = globals.get("on_tool_result")?;
        if hook == Value::Nil {
            return Ok(result.to_string());
        }

        let func: Function = hook
            .as_function()
            .ok_or_else(|| anyhow::anyhow!("on_tool_result is not a function"))?
            .clone();

        // Call hook
        let modified: Value = call_with_timeout(
            || {
                func.call((
                    mcp_name.to_string(),
                    tool_name.to_string(),
                    result.to_string(),
                    is_error,
                ))
            },
            Duration::from_millis(HOOK_TIMEOUT_MS),
        )?;

        // Parse result
        match modified {
            Value::String(s) => Ok(s.to_str()?.to_string()),
            Value::Nil => Ok(result.to_string()),
            _ => Ok(result.to_string()),
        }
    }
}

impl Default for ToolMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

/// Context passed to on_mcp_tools hook
pub struct ToolContext {
    /// Target model (e.g., "qwen-8b") for per-model transforms
    pub target_model: Option<String>,
    /// Model backend (e.g., "llamacpp", "ollama") for backend-specific transforms
    pub model_backend: Option<String>,
}

// ============================================================================
// Lua Registration
// ============================================================================

/// Register tool middleware functions in the tools table
pub fn register_middleware_tools(
    lua: &Lua,
    tools: &Table,
    middleware: ToolMiddleware,
) -> mlua::Result<()> {
    // tools.set_tool_priority({tool = "mcp", ...})
    let set_priority_fn = {
        let middleware = middleware.clone();
        lua.create_function(move |_lua, map: Table| {
            let mut priorities = HashMap::new();
            for pair in map.pairs::<String, String>() {
                let (key, value) = pair?;
                priorities.insert(key, value);
            }
            middleware.set_priorities(priorities);
            Ok(())
        })?
    };
    tools.set("set_tool_priority", set_priority_fn)?;

    // tools.alias_tool(alias, "mcp:tool")
    let alias_fn = {
        let middleware = middleware.clone();
        lua.create_function(move |_lua, (alias, target): (String, String)| {
            middleware.set_alias(&alias, &target);
            Ok(())
        })?
    };
    tools.set("alias_tool", alias_fn)?;

    // tools.get_tool_priorities() -> table
    let get_priorities_fn = {
        let middleware = middleware.clone();
        lua.create_function(move |lua, ()| {
            let priorities = middleware.priorities();
            let table = lua.create_table()?;
            for (tool, mcp) in priorities {
                table.set(tool, mcp)?;
            }
            Ok(table)
        })?
    };
    tools.set("get_tool_priorities", get_priorities_fn)?;

    // tools.get_tool_aliases() -> table
    let get_aliases_fn = {
        let middleware = middleware.clone();
        lua.create_function(move |lua, ()| {
            let aliases = middleware.aliases();
            let table = lua.create_table()?;
            for (alias, target) in aliases {
                table.set(alias, target)?;
            }
            Ok(table)
        })?
    };
    tools.set("get_tool_aliases", get_aliases_fn)?;

    // tools.clear_tool_routing() -> nil
    let clear_fn = {
        lua.create_function(move |_lua, ()| {
            middleware.clear();
            Ok(())
        })?
    };
    tools.set("clear_tool_routing", clear_fn)?;

    Ok(())
}

// ============================================================================
// Conversion Helpers
// ============================================================================

/// Convert ToolInfoExt slice to Lua table
fn tools_to_lua(lua: &Lua, tools: &[ToolInfoExt]) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (i, tool) in tools.iter().enumerate() {
        let tool_table = lua.create_table()?;
        tool_table.set("name", tool.name.as_str())?;
        tool_table.set("description", tool.description.as_str())?;
        tool_table.set("source", tool.source.as_str())?;

        // Convert input_schema to Lua table if present
        if let Some(ref schema) = tool.input_schema {
            let schema_lua = json_to_lua(lua, schema)?;
            tool_table.set("input_schema", schema_lua)?;
        }

        table.set(i + 1, tool_table)?;
    }
    Ok(table)
}

/// Convert Lua table back to ToolInfoExt vec
fn lua_to_tools(table: &Table) -> Result<Vec<ToolInfoExt>> {
    let mut tools = Vec::new();
    for pair in table.clone().pairs::<i64, Table>() {
        let (_, tool_table) = pair?;

        let name: String = tool_table.get("name")?;
        let description: String = tool_table.get("description").unwrap_or_default();
        let source: String = tool_table.get("source").unwrap_or_default();

        // Get input_schema if present
        let input_schema = tool_table
            .get::<Value>("input_schema")
            .ok()
            .and_then(|v| {
                if v == Value::Nil {
                    None
                } else {
                    lua_to_json(&v).ok()
                }
            });

        tools.push(ToolInfoExt {
            name,
            description,
            source,
            input_schema,
        });
    }
    Ok(tools)
}

/// Convert serde_json::Value to Lua Value
fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> mlua::Result<Value> {
    match value {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            // Always use Number (f64) since mlua's Integer is i32
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
                table.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
    }
}

/// Convert Lua Value to serde_json::Value
fn lua_to_json(value: &Value) -> mlua::Result<serde_json::Value> {
    match value {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(i) => Ok(serde_json::Value::Number((*i).into())),
        Value::Number(n) => {
            serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .ok_or_else(|| mlua::Error::runtime("invalid float"))
        }
        Value::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_string())),
        Value::Table(table) => {
            // Check if it's an array (sequential integer keys starting at 1)
            let is_array = table.clone().pairs::<i64, Value>().next().is_some()
                && table
                    .clone()
                    .pairs::<i64, Value>()
                    .enumerate()
                    .all(|(i, pair)| pair.map(|(k, _)| k == (i + 1) as i64).unwrap_or(false));

            if is_array {
                let mut arr = Vec::new();
                for pair in table.clone().pairs::<i64, Value>() {
                    let (_, v) = pair?;
                    arr.push(lua_to_json(&v)?);
                }
                Ok(serde_json::Value::Array(arr))
            } else {
                let mut obj = serde_json::Map::new();
                for pair in table.clone().pairs::<String, Value>() {
                    let (k, v) = pair?;
                    obj.insert(k, lua_to_json(&v)?);
                }
                Ok(serde_json::Value::Object(obj))
            }
        }
        _ => Ok(serde_json::Value::Null),
    }
}

/// Call a function with a timeout
///
/// Note: This is a best-effort timeout. Lua execution is single-threaded,
/// so we can't interrupt mid-execution. The timeout applies to the overall
/// call, but a long-running Lua function won't be interrupted.
fn call_with_timeout<F, T>(f: F, _timeout: Duration) -> Result<T>
where
    F: FnOnce() -> mlua::Result<T>,
{
    // For now, just call directly - true timeout would require separate thread
    // and is complex with Lua's single-threaded nature.
    // The timeout parameter is kept for future enhancement.
    f().map_err(|e| anyhow::anyhow!("Lua call failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_middleware_priority() {
        let mw = ToolMiddleware::new();
        mw.set_priority("sample", "holler");
        mw.set_priority("web_search", "exa");

        let result = mw.resolve_tool("sample");
        assert_eq!(result, Some(("holler".to_string(), "sample".to_string())));

        let result = mw.resolve_tool("unknown");
        assert_eq!(result, None);
    }

    #[test]
    fn test_middleware_alias() {
        let mw = ToolMiddleware::new();
        mw.set_alias("music", "holler:sample");

        let result = mw.resolve_tool("music");
        assert_eq!(result, Some(("holler".to_string(), "sample".to_string())));
    }

    #[test]
    fn test_alias_takes_precedence() {
        let mw = ToolMiddleware::new();
        mw.set_priority("music", "other");
        mw.set_alias("music", "holler:sample");

        // Alias should win
        let result = mw.resolve_tool("music");
        assert_eq!(result, Some(("holler".to_string(), "sample".to_string())));
    }

    #[test]
    fn test_clear_routing() {
        let mw = ToolMiddleware::new();
        mw.set_priority("sample", "holler");
        mw.set_alias("music", "holler:sample");

        mw.clear();

        assert!(mw.priorities().is_empty());
        assert!(mw.aliases().is_empty());
    }

    #[test]
    fn test_json_lua_roundtrip() {
        let lua = Lua::new();
        let original = serde_json::json!({
            "name": "test",
            "count": 42,
            "enabled": true,
            "items": [1, 2, 3],
            "nested": {"foo": "bar"}
        });

        let lua_val = json_to_lua(&lua, &original).unwrap();
        let roundtrip = lua_to_json(&lua_val).unwrap();

        assert_eq!(original, roundtrip);
    }

    #[test]
    fn test_register_middleware_tools() {
        let lua = Lua::new();
        let tools = lua.create_table().unwrap();
        let mw = ToolMiddleware::new();

        register_middleware_tools(&lua, &tools, mw.clone()).unwrap();

        // Set global tools table
        lua.globals().set("tools", tools).unwrap();

        // Test set_tool_priority
        lua.load(r#"
            tools.set_tool_priority({
                sample = "holler",
                web_search = "exa"
            })
        "#)
        .exec()
        .unwrap();

        assert_eq!(mw.priorities().get("sample"), Some(&"holler".to_string()));
        assert_eq!(mw.priorities().get("web_search"), Some(&"exa".to_string()));
    }

    #[test]
    fn test_on_mcp_tools_hook() {
        let lua = Lua::new();
        let mw = ToolMiddleware::new();

        // Define a filtering hook
        lua.load(r#"
            function on_mcp_tools(mcp_name, tools, context)
                local filtered = {}
                for _, tool in ipairs(tools) do
                    if not tool.name:match("^admin_") then
                        table.insert(filtered, tool)
                    end
                end
                return filtered
            end
        "#)
        .exec()
        .unwrap();

        let tools = vec![
            ToolInfo {
                name: "sample".to_string(),
                description: "Sample tool".to_string(),
                source: "test".to_string(),
            },
            ToolInfo {
                name: "admin_delete".to_string(),
                description: "Admin delete".to_string(),
                source: "test".to_string(),
            },
        ];

        let result = mw.process_tools_refresh(&lua, "test", tools, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "sample");
    }

    #[test]
    fn test_on_tool_call_hook() {
        let lua = Lua::new();
        let mw = ToolMiddleware::new();

        // Define a modifying hook
        lua.load(r#"
            function on_tool_call(mcp_name, tool_name, args)
                if not args.creator then
                    args.creator = "hook_default"
                end
                return args
            end
        "#)
        .exec()
        .unwrap();

        let args = serde_json::json!({"space": "orpheus"});
        let result = mw.process_tool_call(&lua, "holler", "sample", args).unwrap();

        assert!(result.is_some());
        let modified = result.unwrap();
        assert_eq!(modified["creator"], "hook_default");
        assert_eq!(modified["space"], "orpheus");
    }

    #[test]
    fn test_on_tool_call_blocking() {
        let lua = Lua::new();
        let mw = ToolMiddleware::new();

        // Define a blocking hook
        lua.load(r#"
            function on_tool_call(mcp_name, tool_name, args)
                if tool_name == "blocked_tool" then
                    return nil  -- Block the call
                end
                return args
            end
        "#)
        .exec()
        .unwrap();

        let args = serde_json::json!({});

        // Allowed tool
        let result = mw
            .process_tool_call(&lua, "test", "allowed_tool", args.clone())
            .unwrap();
        assert!(result.is_some());

        // Blocked tool
        let result = mw
            .process_tool_call(&lua, "test", "blocked_tool", args)
            .unwrap();
        assert!(result.is_none());
    }
}
