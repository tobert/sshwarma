//! Unified tool registry for Lua
//!
//! Provides `sshwarma.call(name, args)` interface where:
//! - Lua handlers take priority (can wrap builtins)
//! - Builtins provide core functionality
//! - MCP tools are available as fallback
//!
//! This replaces the bespoke `tools.X()` functions with a uniform interface.

use anyhow::{anyhow, Result};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;

use crate::db::Database;
use crate::mcp::McpManager;
use crate::state::SharedState;
use crate::status::StatusTracker;
use crate::world::World;
use tokio::sync::RwLock;

/// Context passed to tool handlers
pub struct ToolContext {
    pub db: Arc<Database>,
    pub mcp: Arc<McpManager>,
    pub world: Arc<RwLock<World>>,
    pub status_tracker: Arc<StatusTracker>,
    pub username: Option<String>,
    pub room: Option<String>,
}

impl ToolContext {
    pub fn new(state: &SharedState, status_tracker: Arc<StatusTracker>) -> Self {
        Self {
            db: state.db.clone(),
            mcp: state.mcp.clone(),
            world: state.world.clone(),
            status_tracker,
            username: None,
            room: None,
        }
    }

    pub fn with_user(mut self, username: String) -> Self {
        self.username = Some(username);
        self
    }

    pub fn with_room(mut self, room: String) -> Self {
        self.room = Some(room);
        self
    }
}

/// A builtin tool handler
pub type BuiltinHandler = Box<dyn Fn(&ToolContext, JsonValue) -> Result<JsonValue> + Send + Sync>;

/// Registry of builtin tools
pub struct ToolRegistry {
    handlers: HashMap<String, BuiltinHandler>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            handlers: HashMap::new(),
        };
        registry.register_builtins();
        registry
    }

    /// Register a builtin tool
    pub fn register<F>(&mut self, name: &str, handler: F)
    where
        F: Fn(&ToolContext, JsonValue) -> Result<JsonValue> + Send + Sync + 'static,
    {
        self.handlers.insert(name.to_string(), Box::new(handler));
    }

    /// Call a builtin tool
    pub fn call(&self, name: &str, ctx: &ToolContext, args: JsonValue) -> Result<JsonValue> {
        let handler = self
            .handlers
            .get(name)
            .ok_or_else(|| anyhow!("unknown tool: {}", name))?;
        handler(ctx, args)
    }

    /// Check if a builtin exists
    pub fn has(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    /// List all builtin tool names
    pub fn list(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    /// Register all builtin tools
    fn register_builtins(&mut self) {
        // Status tool - returns live app state for HUD
        self.register("status", |ctx, _args| {
            let mut result = serde_json::json!({});

            // Query room info from DB
            if let Some(ref room_name) = ctx.room {
                let vibe = ctx.db.get_vibe(room_name).ok().flatten();
                let exits = ctx.db.get_exits(room_name).unwrap_or_default();

                result["room"] = serde_json::json!({
                    "name": room_name,
                    "vibe": vibe,
                });

                result["exits"] = serde_json::json!(exits);

                // Try to get participants from world (non-blocking)
                if let Ok(world) = ctx.world.try_read() {
                    if let Some(room) = world.get_room(room_name) {
                        let status_snapshot = ctx.status_tracker.snapshot();

                        let participants: Vec<_> = room
                            .users
                            .iter()
                            .map(|name| {
                                let status = status_snapshot.get(name);
                                serde_json::json!({
                                    "name": name,
                                    "kind": "user",
                                    "status": status.map(|s| s.text()).unwrap_or_default(),
                                    "active": status.map(|s| s.is_active()).unwrap_or(false),
                                })
                            })
                            .chain(room.models.iter().map(|model| {
                                let status = status_snapshot.get(&model.short_name);
                                serde_json::json!({
                                    "name": model.short_name,
                                    "kind": "model",
                                    "status": status.map(|s| s.text()).unwrap_or_default(),
                                    "active": status.map(|s| s.is_active()).unwrap_or(false),
                                })
                            }))
                            .collect();

                        result["participants"] = serde_json::json!(participants);
                    }
                }
            }

            // Session info from status tracker
            result["session"] = serde_json::json!({
                "duration_ms": ctx.status_tracker.duration_ms(),
                "duration": ctx.status_tracker.duration_string(),
            });

            // Add user context
            if let Some(ref user) = ctx.username {
                result["user"] = serde_json::json!(user);
            }

            Ok(result)
        });

        // Room tool - get current room info
        self.register("room", |ctx, _args| {
            if let Some(ref room_name) = ctx.room {
                let vibe = ctx.db.get_vibe(room_name).ok().flatten();
                let exits = ctx.db.get_exits(room_name).unwrap_or_default();

                let (user_count, model_count) = if let Ok(world) = ctx.world.try_read() {
                    if let Some(room) = world.get_room(room_name) {
                        (room.users.len(), room.models.len())
                    } else {
                        (0, 0)
                    }
                } else {
                    (0, 0)
                };

                Ok(serde_json::json!({
                    "name": room_name,
                    "vibe": vibe,
                    "exits": exits,
                    "users": user_count,
                    "models": model_count,
                }))
            } else {
                Ok(serde_json::json!(null))
            }
        });

        // Time tool - get current time info
        self.register("time", |_ctx, _args| {
            let now = chrono::Utc::now();
            Ok(serde_json::json!({
                "unix_ms": now.timestamp_millis(),
                "iso": now.to_rfc3339(),
            }))
        });

        // Screen tool - get terminal dimensions (placeholder)
        self.register("screen", |_ctx, _args| {
            // Will be populated from session state
            Ok(serde_json::json!({
                "width": 80,
                "height": 24,
            }))
        });

        // Notify tool - push a notification
        self.register("notify", |_ctx, args| {
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let _ttl = args.get("ttl").and_then(|v| v.as_i64()).unwrap_or(5000);
            let _level = args.get("level").and_then(|v| v.as_str()).unwrap_or("info");

            // TODO: Actually push notification via channel
            Ok(serde_json::json!({
                "queued": true,
                "message": message,
            }))
        });

        // Dirty tool - mark a region as needing redraw
        self.register("dirty", |_ctx, args| {
            let region = args.get("region").and_then(|v| v.as_str()).unwrap_or("all");

            // TODO: Actually mark dirty via channel
            Ok(serde_json::json!({
                "marked": true,
                "region": region,
            }))
        });

        // Rows tool - get rows from a buffer
        self.register("rows", |ctx, args| {
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20) as usize;

            if let Some(ref room) = ctx.room {
                if let Ok(buffer) = ctx.db.get_or_create_room_buffer(room) {
                    if let Ok(rows) = ctx.db.list_recent_buffer_rows(&buffer.id, limit) {
                        let row_data: Vec<_> = rows
                            .iter()
                            .map(|r| {
                                serde_json::json!({
                                    "id": r.id,
                                    "content_method": r.content_method,
                                    "content": r.content,
                                    "created_at": r.created_at,
                                })
                            })
                            .collect();
                        return Ok(serde_json::json!(row_data));
                    }
                }
            }

            Ok(serde_json::json!([]))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpManager;

    #[test]
    fn test_registry_new() {
        let registry = ToolRegistry::new();
        assert!(registry.has("status"));
        assert!(registry.has("time"));
        assert!(registry.has("room"));
        assert!(!registry.has("nonexistent"));
    }

    #[test]
    fn test_time_tool() {
        let registry = ToolRegistry::new();
        let db = Arc::new(Database::in_memory().unwrap());
        let mcp = Arc::new(McpManager::new());
        let world = Arc::new(RwLock::new(World::new()));
        let status_tracker = Arc::new(StatusTracker::new());

        let ctx = ToolContext {
            db,
            mcp,
            world,
            status_tracker,
            username: None,
            room: None,
        };

        let result = registry.call("time", &ctx, serde_json::json!({})).unwrap();
        assert!(result.get("unix_ms").is_some());
        assert!(result.get("iso").is_some());
    }

    #[test]
    fn test_list_tools() {
        let registry = ToolRegistry::new();
        let tools = registry.list();
        assert!(tools.contains(&"status".to_string()));
        assert!(tools.contains(&"time".to_string()));
    }
}
