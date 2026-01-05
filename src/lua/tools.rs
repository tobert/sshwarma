//! Tool bridge for Lua HUD
//!
//! Provides Lua functions that bridge to Rust state and MCP tools.
//! All functions are registered in a `tools` global table.

use crate::lua::cache::ToolCache;
use crate::lua::context::{build_notifications_table, NotificationLevel, PendingNotification};
use crate::lua::dirty::DirtyState;
use crate::lua::mcp_bridge::McpBridge;
use crate::lua::tool_middleware::ToolMiddleware;
use crate::model::ModelHandle;
use crate::state::SharedState;
use crate::status::{Status, StatusTracker};
use crate::ui::{LuaArea, LuaDrawContext, Rect, RenderBuffer};
use crate::world::JournalKind;
use mlua::{Lua, Result as LuaResult, Table, UserData, UserDataMethods, Value};
use std::sync::Arc;
use unicode_width::UnicodeWidthStr;

/// Session context for wrap operations
///
/// Provides information about the current user, model, and room
/// for context composition during @mention processing.
#[derive(Clone)]
pub struct SessionContext {
    /// Username of the person who triggered the interaction
    pub username: String,
    /// Model being addressed (if any)
    pub model: Option<ModelHandle>,
    /// Current room name (None if in lobby)
    pub room_name: Option<String>,
}

/// Current input line state for Lua rendering
#[derive(Clone, Default)]
pub struct InputState {
    /// Current input text
    pub text: String,
    /// Cursor position (byte offset)
    pub cursor: usize,
    /// Prompt string (e.g., "lobby> ")
    pub prompt: String,
}

/// Region content for overlay regions (help, command output, etc.)
///
/// Stored per-region-name. Visibility is controlled via Lua regions module.
#[derive(Clone, Default)]
pub struct RegionContent {
    /// Title shown at top of region (e.g., "Help")
    pub title: String,
    /// Content lines (pre-split for rendering)
    pub lines: Vec<String>,
    /// Current scroll offset (line index of first visible line)
    pub scroll_offset: usize,
}

/// Shared state holder for Lua callbacks
///
/// Uses Arc for thread-safe sharing across async handlers and
/// spawned tasks (required for russh's Send+Sync handler bounds).
#[derive(Clone)]
pub struct LuaToolState {
    /// Participant status tracker (thinking, running tool, etc.)
    status_tracker: Arc<StatusTracker>,
    /// Pending notifications queue (Rust adds, Lua drains)
    pending_notifications: Arc<std::sync::RwLock<Vec<PendingNotification>>>,
    /// Tool result cache for instant reads
    cache: ToolCache,
    /// Shared application state for extended data access (world, ledger, etc.)
    shared_state: Arc<std::sync::RwLock<Option<Arc<SharedState>>>>,
    /// Session context (user, model, room) for wrap operations
    session_context: Arc<std::sync::RwLock<Option<SessionContext>>>,
    /// Tool middleware for routing and transformation
    middleware: ToolMiddleware,
    /// Current input line state (for Lua to render)
    input_state: Arc<std::sync::RwLock<InputState>>,
    /// Tag-based dirty tracking for partial screen updates
    /// Lua defines regions; Rust provides primitives
    dirty: Arc<DirtyState>,
    /// Chat scroll state (persists across renders)
    chat_scroll: crate::ui::scroll::LuaScrollState,
    /// Region contents (keyed by region name like "overlay", "help", etc.)
    /// Content is stored here, visibility is managed via Lua regions module
    region_contents: Arc<std::sync::RwLock<std::collections::HashMap<String, RegionContent>>>,
}

impl LuaToolState {
    /// Create a new tool state
    pub fn new() -> Self {
        Self {
            status_tracker: Arc::new(StatusTracker::new()),
            pending_notifications: Arc::new(std::sync::RwLock::new(Vec::new())),
            cache: ToolCache::new(),
            shared_state: Arc::new(std::sync::RwLock::new(None)),
            session_context: Arc::new(std::sync::RwLock::new(None)),
            middleware: ToolMiddleware::new(),
            input_state: Arc::new(std::sync::RwLock::new(InputState::default())),
            dirty: Arc::new(DirtyState::new()),
            chat_scroll: crate::ui::scroll::LuaScrollState::new(),
            region_contents: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Get the chat scroll state (for Lua HUD)
    pub fn chat_scroll(&self) -> crate::ui::scroll::LuaScrollState {
        self.chat_scroll.clone()
    }

    /// Get the dirty state for screen refresh task
    ///
    /// Screen loop waits on `dirty.notified()` and calls `dirty.take()` to get dirty tags.
    pub fn dirty(&self) -> &Arc<DirtyState> {
        &self.dirty
    }

    /// Mark a tag dirty for partial screen updates
    ///
    /// Conventional tags: "status", "chat", "input"
    /// Lua can define its own tag names for custom layouts.
    pub fn mark_dirty(&self, tag: &str) {
        self.dirty.mark(tag);
    }

    /// Update the current input state (called by handler on keystroke)
    pub fn set_input(&self, text: &str, cursor: usize, prompt: &str) {
        if let Ok(mut guard) = self.input_state.write() {
            guard.text = text.to_string();
            guard.cursor = cursor;
            guard.prompt = prompt.to_string();
        }
        self.mark_dirty("input");
    }

    /// Get the current input state
    pub fn input_state(&self) -> InputState {
        self.input_state
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Show content in a named region (overlay, help, etc.)
    ///
    /// Content is split into lines for scrolling. Sets content and marks region visible.
    /// Visibility is controlled via the Lua regions module.
    pub fn show_region(&self, region: &str, title: &str, content: &str) {
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        if let Ok(mut guard) = self.region_contents.write() {
            guard.insert(
                region.to_string(),
                RegionContent {
                    title: title.to_string(),
                    lines,
                    scroll_offset: 0,
                },
            );
        }
        self.mark_dirty(region);
    }

    /// Hide a region and clear its content
    pub fn hide_region(&self, region: &str) {
        if let Ok(mut guard) = self.region_contents.write() {
            guard.remove(region);
        }
        self.mark_dirty(region);
    }

    /// Check if a region has content
    pub fn has_region_content(&self, region: &str) -> bool {
        self.region_contents
            .read()
            .map(|guard| guard.contains_key(region))
            .unwrap_or(false)
    }

    /// Get a region's content
    pub fn region_content(&self, region: &str) -> Option<RegionContent> {
        self.region_contents
            .read()
            .ok()
            .and_then(|guard| guard.get(region).cloned())
    }

    /// Scroll a region up by n lines
    pub fn region_scroll_up(&self, region: &str, n: usize) {
        if let Ok(mut guard) = self.region_contents.write() {
            if let Some(ref mut content) = guard.get_mut(region) {
                content.scroll_offset = content.scroll_offset.saturating_sub(n);
            }
        }
        self.mark_dirty(region);
    }

    /// Scroll a region down by n lines
    pub fn region_scroll_down(&self, region: &str, n: usize, viewport_height: usize) {
        if let Ok(mut guard) = self.region_contents.write() {
            if let Some(ref mut content) = guard.get_mut(region) {
                let max_scroll = content.lines.len().saturating_sub(viewport_height);
                content.scroll_offset = (content.scroll_offset + n).min(max_scroll);
            }
        }
        self.mark_dirty(region);
    }

    /// Compatibility wrapper: show overlay (uses "overlay" region)
    pub fn show_overlay(&self, title: &str, content: &str) {
        self.show_region("overlay", title, content);
    }

    /// Compatibility wrapper: close overlay (hides "overlay" region)
    pub fn close_overlay(&self) {
        self.hide_region("overlay");
    }

    /// Compatibility wrapper: check if overlay has content
    pub fn has_overlay(&self) -> bool {
        self.has_region_content("overlay")
    }

    /// Compatibility wrapper: scroll overlay up
    pub fn overlay_scroll_up(&self, n: usize) {
        self.region_scroll_up("overlay", n);
    }

    /// Compatibility wrapper: scroll overlay down
    pub fn overlay_scroll_down(&self, n: usize, viewport_height: usize) {
        self.region_scroll_down("overlay", n, viewport_height);
    }

    /// Get a reference to the tool middleware
    pub fn middleware(&self) -> &ToolMiddleware {
        &self.middleware
    }

    /// Get the status tracker
    pub fn status_tracker(&self) -> &Arc<StatusTracker> {
        &self.status_tracker
    }

    /// Set a participant's status
    pub fn set_status(&self, name: &str, status: Status) {
        self.status_tracker.set(name, status);
        self.mark_dirty("status");
    }

    /// Get a participant's status
    pub fn get_status(&self, name: &str) -> Status {
        self.status_tracker.get(name)
    }

    /// Push a notification to the queue
    pub fn push_notification(&self, message: String, ttl_ms: i64) {
        self.push_notification_with_level(message, ttl_ms, NotificationLevel::Info);
    }

    /// Push a notification with a specific level
    pub fn push_notification_with_level(
        &self,
        message: String,
        ttl_ms: i64,
        level: NotificationLevel,
    ) {
        let notification = PendingNotification {
            message,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            ttl_ms,
            level,
        };
        if let Ok(mut guard) = self.pending_notifications.write() {
            guard.push(notification);
        }
        self.mark_dirty("status");
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

    /// Set the shared state for extended data access
    ///
    /// Call this before wrap operations that need access to world, ledger, etc.
    pub fn set_shared_state(&self, state: Option<Arc<SharedState>>) {
        if let Ok(mut guard) = self.shared_state.write() {
            *guard = state;
        }
    }

    /// Get a clone of the shared state if set
    pub fn shared_state(&self) -> Option<Arc<SharedState>> {
        self.shared_state
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Set the session context for wrap operations
    pub fn set_session_context(&self, ctx: Option<SessionContext>) {
        if let Ok(mut guard) = self.session_context.write() {
            let room = ctx.as_ref().and_then(|c| c.room_name.clone());
            tracing::info!(?room, "set_session_context");
            *guard = ctx;
        }
        // Room/context change affects all regions
        self.dirty.mark_many(["status", "chat", "input"]);
    }

    /// Get a clone of the session context if set
    pub fn session_context(&self) -> Option<SessionContext> {
        let ctx = self
            .session_context
            .read()
            .ok()
            .and_then(|guard| guard.clone());
        let room = ctx.as_ref().and_then(|c| c.room_name.clone());
        tracing::debug!(?room, "session_context read");
        ctx
    }

    /// Clear the session context (cleanup after wrap operations)
    pub fn clear_session_context(&self) {
        self.set_session_context(None);
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
/// - `tools.clear_notifications()` - drains pending notifications
/// - `tools.cached(key)` - reads from cache
///
/// For HUD state, use `sshwarma.call("status")` instead.
pub fn register_tools(lua: &Lua, state: LuaToolState) -> LuaResult<()> {
    let tools = lua.create_table()?;

    // Store state in Lua registry for access from callbacks
    lua.set_named_registry_value("tool_state", LuaToolStateWrapper(state.clone()))?;

    // tools.clear_notifications() -> array of notifications
    let clear_notifications_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let notifications = state.drain_notifications();
            if notifications.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::Table(build_notifications_table(
                    lua,
                    &notifications,
                )?))
            }
        })?
    };
    tools.set("clear_notifications", clear_notifications_fn)?;

    // tools.notify(message, [level], [ttl_ms]) -> nil
    // Pushes a notification to the queue
    // level: "info" (default), "warning", "error"
    // ttl_ms: time-to-live in milliseconds (default: 5000)
    let notify_fn = {
        let state = state.clone();
        lua.create_function(
            move |_lua, (message, level, ttl_ms): (String, Option<String>, Option<i64>)| {
                let level = match level.as_deref() {
                    Some("error") => NotificationLevel::Error,
                    Some("warning") => NotificationLevel::Warning,
                    _ => NotificationLevel::Info,
                };
                let ttl = ttl_ms.unwrap_or(5000);
                state.push_notification_with_level(message, ttl, level);
                Ok(())
            },
        )?
    };
    tools.set("notify", notify_fn)?;

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

    // Room/participant tools (query live data from session_context, shared_state, status_tracker)

    // tools.look() -> room summary
    let look_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let result = lua.create_table()?;

            // Get room name from session context
            let room_name = state.session_context().and_then(|ctx| ctx.room_name);

            if let Some(ref room) = room_name {
                result.set("room", room.clone())?;
            } else {
                result.set("room", Value::Nil)?;
                return Ok(result);
            }

            let room_name = room_name.unwrap();

            // Get room data from shared state
            if let Some(shared) = state.shared_state() {
                // Get room from world
                let world = tokio::task::block_in_place(|| shared.world.blocking_read());
                if let Some(room) = world.get_room(&room_name) {
                    // Description
                    if let Some(ref desc) = room.description {
                        result.set("description", desc.clone())?;
                    } else {
                        result.set("description", Value::Nil)?;
                    }

                    // Users array
                    let users_table = lua.create_table()?;
                    for (i, user) in room.users.iter().enumerate() {
                        users_table.set(i + 1, user.clone())?;
                    }
                    result.set("users", users_table)?;
                }
                drop(world);

                // Get vibe from DB
                let vibe = shared.db.get_vibe(&room_name).ok().flatten();
                if let Some(v) = vibe {
                    result.set("vibe", v)?;
                } else {
                    result.set("vibe", Value::Nil)?;
                }

                // Get exits from DB
                let exits = shared.db.get_exits(&room_name).unwrap_or_default();
                let exits_table = lua.create_table()?;
                for (dir, dest) in exits {
                    exits_table.set(dir, dest)?;
                }
                result.set("exits", exits_table)?;

                // Models array (from model registry)
                let models_table = lua.create_table()?;
                for (i, m) in shared.models.available().iter().enumerate() {
                    models_table.set(i + 1, m.short_name.clone())?;
                }
                result.set("models", models_table)?;
            }

            Ok(result)
        })?
    };
    tools.set("look", look_fn)?;

    // tools.who() -> participant list with status
    let who_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let list = lua.create_table()?;
            let mut idx = 1;

            // Get room name from session context
            let room_name = state.session_context().and_then(|ctx| ctx.room_name);

            if let Some(shared) = state.shared_state() {
                // Get users from room
                if let Some(ref room) = room_name {
                    let world = tokio::task::block_in_place(|| shared.world.blocking_read());
                    if let Some(room_data) = world.get_room(room) {
                        for user in &room_data.users {
                            let entry = lua.create_table()?;
                            entry.set("name", user.clone())?;
                            entry.set("is_model", false)?;
                            let status = state.get_status(user);
                            entry.set("status", status.text())?;
                            entry.set("glyph", status.glyph())?;
                            list.set(idx, entry)?;
                            idx += 1;
                        }
                    }
                }

                // Get models from registry
                for m in shared.models.available() {
                    let entry = lua.create_table()?;
                    entry.set("name", m.short_name.clone())?;
                    entry.set("is_model", true)?;
                    let status = state.get_status(&m.short_name);
                    entry.set("status", status.text())?;
                    entry.set("glyph", status.glyph())?;
                    list.set(idx, entry)?;
                    idx += 1;
                }
            }

            Ok(list)
        })?
    };
    tools.set("who", who_fn)?;

    // tools.exits() -> direction to destination map
    let exits_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let map = lua.create_table()?;

            // Get room name from session context
            let room_name = state.session_context().and_then(|ctx| ctx.room_name);

            if let (Some(shared), Some(room)) = (state.shared_state(), room_name) {
                let exits = shared.db.get_exits(&room).unwrap_or_default();
                for (dir, dest) in exits {
                    map.set(dir, dest)?;
                }
            }

            Ok(map)
        })?
    };
    tools.set("exits", exits_fn)?;

    // tools.vibe() -> string or nil
    let vibe_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, ()| {
            // Get room name from session context
            let room_name = state.session_context().and_then(|ctx| ctx.room_name);

            if let (Some(shared), Some(room)) = (state.shared_state(), room_name) {
                Ok(shared.db.get_vibe(&room).ok().flatten())
            } else {
                Ok(None)
            }
        })?
    };
    tools.set("vibe", vibe_fn)?;

    // tools.mcp_connections() -> MCP connections
    let mcp_connections_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let list = lua.create_table()?;

            if let Some(shared) = state.shared_state() {
                for (i, server) in shared.mcp.list().iter().enumerate() {
                    let entry = lua.create_table()?;
                    entry.set("name", server.name.clone())?;
                    entry.set("tools", server.tool_count)?;
                    entry.set("connected", server.state == "connected")?;
                    entry.set("calls", server.call_count as i64)?;
                    if let Some(ref last_tool) = server.last_tool {
                        entry.set("last_tool", last_tool.clone())?;
                    }
                    list.set(i + 1, entry)?;
                }
            }

            Ok(list)
        })?
    };
    tools.set("mcp_connections", mcp_connections_fn)?;

    // tools.session() -> session info
    let session_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let result = lua.create_table()?;
            let tracker = state.status_tracker();
            result.set("start_ms", tracker.session_start().timestamp_millis())?;
            result.set("duration", tracker.duration_string())?;
            // Spinner frame: tick-based, advances every 100ms
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let spinner_frame = ((now / 100) % 10) as u8;
            result.set("spinner_frame", spinner_frame)?;
            Ok(result)
        })?
    };
    tools.set("session", session_fn)?;

    // tools.input() -> {text, cursor, prompt}
    let input_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let input = state.input_state();
            let result = lua.create_table()?;
            result.set("text", input.text)?;
            result.set("cursor", input.cursor)?;
            result.set("prompt", input.prompt)?;
            Ok(result)
        })?
    };
    tools.set("input", input_fn)?;

    // tools.set_input(text, cursor, prompt) -> nil
    // Update the input state (called from Lua input module)
    let set_input_fn = {
        let state = state.clone();
        lua.create_function(
            move |_lua, (text, cursor, prompt): (String, usize, Option<String>)| {
                let prompt = prompt.unwrap_or_else(|| "> ".to_string());
                state.set_input(&text, cursor, &prompt);
                Ok(())
            },
        )?
    };
    tools.set("set_input", set_input_fn)?;

    // tools.scroll() -> persistent LuaScrollState
    // Returns the same scroll state across renders so position is maintained
    let scroll_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, ()| Ok(state.chat_scroll()))?
    };
    tools.set("scroll", scroll_fn)?;

    // tools.mark_dirty(tag) -> nil
    // Mark a region tag dirty for partial screen updates
    // Conventional tags: "status", "chat", "input" (Lua can define others)
    let mark_dirty_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, tag: String| {
            state.mark_dirty(&tag);
            Ok(())
        })?
    };
    tools.set("mark_dirty", mark_dirty_fn)?;

    // tools.mark_all_dirty(tags) -> nil
    // Mark multiple region tags dirty at once
    let mark_all_dirty_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, tags: Vec<String>| {
            state.dirty().mark_many(tags);
            Ok(())
        })?
    };
    tools.set("mark_all_dirty", mark_all_dirty_fn)?;

    // tools.region_content(name) -> {title, lines, scroll_offset, total_lines} or nil
    // Get content for a named region
    let region_content_fn = {
        let state = state.clone();
        lua.create_function(move |lua, name: String| {
            if let Some(content) = state.region_content(&name) {
                let result = lua.create_table()?;
                result.set("title", content.title)?;

                let lines_table = lua.create_table()?;
                for (i, line) in content.lines.iter().enumerate() {
                    lines_table.set(i + 1, line.clone())?;
                }
                result.set("lines", lines_table)?;
                result.set("scroll_offset", content.scroll_offset)?;
                result.set("total_lines", content.lines.len())?;

                Ok(Value::Table(result))
            } else {
                Ok(Value::Nil)
            }
        })?
    };
    tools.set("region_content", region_content_fn)?;

    // tools.hide_region(name) -> nil
    // Hide a region and clear its content
    let hide_region_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, name: String| {
            state.hide_region(&name);
            Ok(())
        })?
    };
    tools.set("hide_region", hide_region_fn)?;

    // Backwards compatibility: tools.overlay() uses "overlay" region
    let overlay_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            if let Some(content) = state.region_content("overlay") {
                let result = lua.create_table()?;
                result.set("title", content.title)?;

                let lines_table = lua.create_table()?;
                for (i, line) in content.lines.iter().enumerate() {
                    lines_table.set(i + 1, line.clone())?;
                }
                result.set("lines", lines_table)?;
                result.set("scroll_offset", content.scroll_offset)?;
                result.set("total_lines", content.lines.len())?;

                Ok(Value::Table(result))
            } else {
                Ok(Value::Nil)
            }
        })?
    };
    tools.set("overlay", overlay_fn)?;

    // Backwards compatibility: tools.close_overlay() hides "overlay" region
    let close_overlay_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, ()| {
            state.hide_region("overlay");
            Ok(())
        })?
    };
    tools.set("close_overlay", close_overlay_fn)?;

    // Extended data tools (require SharedState)

    // tools.history(opts) -> [{author, content, timestamp, kind}]
    // opts can be:
    //   - number: limit (backward compat)
    //   - table: {limit, agents, thread, since_marker}
    let history_fn = {
        let state = state.clone();
        lua.create_function(move |lua, opts: Value| {
            let list = lua.create_table()?;

            // Parse options - either number or table
            let (limit, agent_filter): (usize, Option<Vec<String>>) = match opts {
                Value::Nil => (30, None),
                Value::Integer(n) => (n.max(1) as usize, None),
                Value::Number(n) => (n.max(1.0) as usize, None),
                Value::Table(ref tbl) => {
                    let limit: usize = tbl.get::<usize>("limit").unwrap_or(30);
                    let agents: Option<Vec<String>> = tbl.get("agents").ok();
                    // thread and since_marker reserved for future
                    (limit, agents)
                }
                _ => (30, None),
            };

            // Get room name from session context
            let ctx = state.session_context();
            let room_name = match ctx.and_then(|c| c.room_name) {
                Some(name) => {
                    tracing::debug!(room = %name, "tools.history: found room");
                    name
                }
                None => {
                    tracing::debug!("tools.history: no room in session context");
                    return Ok(list); // Empty list if not in a room
                }
            };

            // Try to get history from database
            if let Some(shared) = state.shared_state() {
                // Get buffer for room
                if let Ok(buffer) = shared.db.get_or_create_room_buffer(&room_name) {
                    // Get recent rows (fetch more than limit to account for filtering)
                    let fetch_limit = if agent_filter.is_some() {
                        limit * 3
                    } else {
                        limit
                    };
                    if let Ok(rows) = shared.db.list_recent_buffer_rows(&buffer.id, fetch_limit) {
                        let mut idx = 1;
                        let mut count = 0;
                        for db_row in rows.iter().filter(|r| !r.ephemeral) {
                            if count >= limit {
                                break;
                            }

                            // Include message rows and thinking/streaming rows
                            let is_message = db_row.content_method.starts_with("message.");
                            let is_thinking = db_row.content_method.starts_with("thinking.");
                            if !is_message && !is_thinking {
                                continue;
                            }

                            // Get content
                            let text = match &db_row.content {
                                Some(t) => t.clone(),
                                None => continue,
                            };

                            // Get author name from agent
                            let author = if let Some(ref agent_id) = db_row.source_agent_id {
                                if let Ok(Some(agent)) = shared.db.get_agent(agent_id) {
                                    agent.name
                                } else {
                                    "unknown".to_string()
                                }
                            } else {
                                "system".to_string()
                            };

                            // Apply agent filter if specified
                            if let Some(ref agents) = agent_filter {
                                if !agents.iter().any(|a| a == &author) {
                                    continue;
                                }
                            }

                            let row = lua.create_table()?;
                            row.set("author", author)?;
                            row.set("content", text)?;
                            row.set("timestamp", db_row.created_at)?;
                            row.set(
                                "is_model",
                                is_message && db_row.content_method == "message.model"
                                    || is_thinking,
                            )?;
                            row.set("is_thinking", is_thinking)?;
                            row.set("is_streaming", is_thinking && db_row.finalized_at.is_none())?;
                            list.set(idx, row)?;
                            idx += 1;
                            count += 1;
                        }
                    }
                }
            }

            Ok(list)
        })?
    };
    tools.set("history", history_fn)?;

    // tools.history_tools(limit?) -> [{tool, args, result, success, timestamp}]
    // Returns recent MCP tool calls
    let history_tools_fn = {
        let state = state.clone();
        lua.create_function(move |lua, limit: Option<usize>| {
            let limit = limit.unwrap_or(20);
            let list = lua.create_table()?;

            // Get room from session context
            let ctx = state.session_context();
            let room_name = match ctx.and_then(|c| c.room_name) {
                Some(name) => name,
                None => return Ok(list),
            };

            if let Some(shared) = state.shared_state() {
                // Get buffer for room
                if let Ok(buffer) = shared.db.get_or_create_room_buffer(&room_name) {
                    // Get recent tool calls
                    if let Ok(rows) = shared.db.list_tool_calls(&buffer.id, limit) {
                        let mut idx = 1;
                        let mut current_call: Option<mlua::Table> = None;
                        let mut current_tool: Option<String> = None;

                        for row in rows {
                            if row.content_method == "tool.call" {
                                // Start a new tool call entry
                                if let Some(call) = current_call.take() {
                                    list.set(idx, call)?;
                                    idx += 1;
                                }

                                let entry = lua.create_table()?;
                                let tool_name = row.content.clone().unwrap_or_default();
                                entry.set("tool", tool_name.clone())?;
                                entry.set("timestamp", row.created_at)?;

                                // Get args from content_meta if available
                                if let Some(ref meta) = row.content_meta {
                                    if let Ok(parsed) =
                                        serde_json::from_str::<serde_json::Value>(meta)
                                    {
                                        if let Some(args) = parsed.get("input") {
                                            entry.set("args", args.to_string())?;
                                        }
                                    }
                                }

                                current_tool = Some(tool_name);
                                current_call = Some(entry);
                            } else if row.content_method == "tool.result" {
                                // Attach result to current call if tool names match
                                if let (Some(ref call), Some(ref tool)) =
                                    (&current_call, &current_tool)
                                {
                                    let result_tool = row.content.clone().unwrap_or_default();
                                    if result_tool == *tool {
                                        if let Some(ref meta) = row.content_meta {
                                            if let Ok(parsed) =
                                                serde_json::from_str::<serde_json::Value>(meta)
                                            {
                                                if let Some(result) = parsed.get("result") {
                                                    let result_str = match result {
                                                        serde_json::Value::String(s) => s.clone(),
                                                        _ => result.to_string(),
                                                    };
                                                    call.set("result", result_str)?;
                                                }
                                                if let Some(success) = parsed.get("success") {
                                                    call.set(
                                                        "success",
                                                        success.as_bool().unwrap_or(true),
                                                    )?;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Don't forget the last call
                        if let Some(call) = current_call.take() {
                            list.set(idx, call)?;
                        }
                    }
                }
            }

            Ok(list)
        })?
    };
    tools.set("history_tools", history_tools_fn)?;

    // tools.history_stats() -> {total, tools = [{name, count}]}
    // Returns tool usage statistics
    let history_stats_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let result = lua.create_table()?;
            result.set("total", 0)?;

            // Get room from session context
            let ctx = state.session_context();
            let room_name = match ctx.and_then(|c| c.room_name) {
                Some(name) => name,
                None => return Ok(result),
            };

            if let Some(shared) = state.shared_state() {
                // Get buffer for room
                if let Ok(buffer) = shared.db.get_or_create_room_buffer(&room_name) {
                    // Get tool call counts
                    if let Ok(counts) = shared.db.count_tool_calls(&buffer.id) {
                        let tools_table = lua.create_table()?;
                        let mut total = 0usize;
                        let mut idx = 1;

                        for (tool_name, count) in counts {
                            let entry = lua.create_table()?;
                            entry.set("name", tool_name)?;
                            entry.set("count", count)?;
                            tools_table.set(idx, entry)?;
                            idx += 1;
                            total += count;
                        }

                        result.set("total", total)?;
                        result.set("tools", tools_table)?;
                    }
                }
            }

            Ok(result)
        })?
    };
    tools.set("history_stats", history_stats_fn)?;

    // tools.journal(kind, n) -> [{kind, author, content, timestamp}]
    let journal_fn = {
        let state = state.clone();
        lua.create_function(move |lua, (kind, limit): (Option<String>, Option<usize>)| {
            let limit = limit.unwrap_or(10);
            let list = lua.create_table()?;

            // Parse kind filter if provided
            let kind_filter = kind.as_ref().and_then(|k| JournalKind::parse(k));

            // Get room name from session context
            let room_name = match state.session_context().and_then(|ctx| ctx.room_name) {
                Some(name) => name,
                None => return Ok(list),
            };

            // Try to get journal from SharedState
            if let Some(shared) = state.shared_state() {
                let world = tokio::task::block_in_place(|| shared.world.blocking_read());
                if let Some(room) = world.rooms.get(&room_name) {
                    let mut idx = 1;
                    let entries: Vec<_> = room
                        .context
                        .journal
                        .iter()
                        .filter(|e| kind_filter.is_none_or(|k| e.kind == k))
                        .rev()
                        .take(limit)
                        .collect();

                    for entry in entries.into_iter().rev() {
                        let row = lua.create_table()?;
                        row.set("kind", format!("{}", entry.kind))?;
                        row.set("author", entry.author.clone())?;
                        row.set("content", entry.content.clone())?;
                        row.set("timestamp", entry.timestamp.timestamp_millis())?;
                        list.set(idx, row)?;
                        idx += 1;
                    }
                }
            }

            Ok(list)
        })?
    };
    tools.set("journal", journal_fn)?;

    // tools.inspirations() -> [{content}]
    let inspirations_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let list = lua.create_table()?;

            // Get room name from session context
            let room_name = match state.session_context().and_then(|ctx| ctx.room_name) {
                Some(name) => name,
                None => return Ok(list),
            };

            // Try to get inspirations from SharedState
            if let Some(shared) = state.shared_state() {
                let world = tokio::task::block_in_place(|| shared.world.blocking_read());
                if let Some(room) = world.rooms.get(&room_name) {
                    for (i, insp) in room.context.inspirations.iter().enumerate() {
                        let row = lua.create_table()?;
                        row.set("content", insp.content.clone())?;
                        list.set(i + 1, row)?;
                    }
                }
            }

            Ok(list)
        })?
    };
    tools.set("inspirations", inspirations_fn)?;

    // tools.rooms() -> [{name, user_count, model_count, description}]
    let rooms_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let list = lua.create_table()?;

            if let Some(shared) = state.shared_state() {
                let world = tokio::task::block_in_place(|| shared.world.blocking_read());
                let mut idx = 1;
                for (name, room) in &world.rooms {
                    let row = lua.create_table()?;
                    row.set("name", name.clone())?;
                    row.set("user_count", room.users.len())?;
                    row.set("model_count", room.models.len())?;
                    if let Some(ref desc) = room.description {
                        row.set("description", desc.clone())?;
                    }
                    list.set(idx, row)?;
                    idx += 1;
                }
            }

            Ok(list)
        })?
    };
    tools.set("rooms", rooms_fn)?;

    // tools.current_user() -> {name} or nil
    let current_user_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            if let Some(ctx) = state.session_context() {
                let result = lua.create_table()?;
                result.set("name", ctx.username)?;
                Ok(Value::Table(result))
            } else {
                Ok(Value::Nil)
            }
        })?
    };
    tools.set("current_user", current_user_fn)?;

    // tools.current_model() -> {name, display, ...} or nil
    let current_model_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            if let Some(ctx) = state.session_context() {
                if let Some(model) = ctx.model {
                    let result = lua.create_table()?;
                    result.set("name", model.short_name.clone())?;
                    result.set("display", model.display_name.clone())?;
                    if let Some(ref system_prompt) = model.system_prompt {
                        result.set("system_prompt", system_prompt.clone())?;
                    }
                    if let Some(context_window) = model.context_window {
                        result.set("context_window", context_window)?;
                    }
                    return Ok(Value::Table(result));
                }
            }
            Ok(Value::Nil)
        })?
    };
    tools.set("current_model", current_model_fn)?;

    // tools.get_target_prompts(target) -> [{slot, prompt_name, content}]
    // Returns resolved prompts for a target (model or user) using new prompt system
    let get_target_prompts_fn = {
        let state = state.clone();
        lua.create_function(move |lua, target: String| {
            let list = lua.create_table()?;

            // Get room name from session context
            let room_name = match state.session_context().and_then(|ctx| ctx.room_name) {
                Some(name) => name,
                None => return Ok(list),
            };

            // Get target slots from database
            if let Some(shared) = state.shared_state() {
                match shared.db.get_target_slots(&room_name, &target) {
                    Ok(slots) => {
                        for (i, slot) in slots.iter().enumerate() {
                            let row = lua.create_table()?;
                            row.set("slot", slot.index)?;
                            row.set("prompt_name", slot.prompt_name.clone())?;
                            row.set("target_type", slot.target_type.clone())?;
                            if let Some(ref content) = slot.content {
                                row.set("content", content.clone())?;
                            }
                            list.set(i + 1, row)?;
                        }
                    }
                    Err(_) => {
                        // Return empty list on error
                    }
                }
            }

            Ok(list)
        })?
    };
    tools.set("get_target_prompts", get_target_prompts_fn)?;

    // Utility tools

    // tools.display_width(text) -> int
    // Returns terminal display width of text (handles wide CJK chars, zero-width, etc.)
    // Uses Unicode UAX#11 East Asian Width property
    let display_width_fn = lua.create_function(|_, text: String| Ok(text.width()))?;
    tools.set("display_width", display_width_fn)?;

    // tools.estimate_tokens(text) -> int
    // Simple heuristic: ~4 characters per token
    let estimate_tokens_fn = lua.create_function(|_, text: String| Ok(text.len() / 4))?;
    tools.set("estimate_tokens", estimate_tokens_fn)?;

    // tools.truncate(text, max_tokens) -> string
    // Truncates text to approximately max_tokens
    let truncate_fn = lua.create_function(|_, (text, max_tokens): (String, usize)| {
        let max_chars = max_tokens * 4; // Reverse of estimate
        if text.len() <= max_chars {
            Ok(text)
        } else {
            // Find word boundary near truncation point
            let truncated = &text[..max_chars.min(text.len())];
            if let Some(last_space) = truncated.rfind(' ') {
                Ok(format!("{}...", &truncated[..last_space]))
            } else {
                Ok(format!("{}...", truncated))
            }
        }
    })?;
    tools.set("truncate", truncate_fn)?;

    // MCP management tools (non-blocking, control plane)

    // tools.mcp_add(name, url) -> nil
    // Add connection (non-blocking, idempotent). Connection happens in background.
    let mcp_add_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, (name, url): (String, String)| {
            if let Some(shared) = state.shared_state() {
                shared.mcp.add(&name, &url); // Non-blocking, spawns task
            }
            Ok(())
        })?
    };
    tools.set("mcp_add", mcp_add_fn)?;

    // tools.mcp_remove(name) -> bool
    // Remove connection (graceful disconnect). Returns true if was present.
    let mcp_remove_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, name: String| {
            match state.shared_state() {
                Some(shared) => Ok(shared.mcp.remove(&name)), // Non-blocking
                None => Ok(false),
            }
        })?
    };
    tools.set("mcp_remove", mcp_remove_fn)?;

    // tools.mcp_status(name) -> table or nil
    // Get status of one connection.
    // Returns: { name, state, tools, error, attempt } or nil if not found
    let mcp_status_fn = {
        let state = state.clone();
        lua.create_function(move |lua, name: String| {
            let Some(shared) = state.shared_state() else {
                return Ok(Value::Nil);
            };
            match shared.mcp.status(&name) {
                Some(status) => {
                    let table = lua.create_table()?;
                    table.set("name", status.name)?;
                    table.set("state", status.state)?;
                    table.set("tools", status.tool_count)?;
                    table.set("endpoint", status.endpoint)?;
                    table.set("calls", status.call_count)?;
                    if let Some(err) = status.error {
                        table.set("error", err)?;
                    }
                    if let Some(attempt) = status.attempt {
                        table.set("attempt", attempt)?;
                    }
                    if let Some(last_tool) = status.last_tool {
                        table.set("last_tool", last_tool)?;
                    }
                    Ok(Value::Table(table))
                }
                None => Ok(Value::Nil),
            }
        })?
    };
    tools.set("mcp_status", mcp_status_fn)?;

    // tools.mcp_list() -> array of status tables
    // List all connections with their status.
    let mcp_list_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let Some(shared) = state.shared_state() else {
                return lua.create_table(); // Empty table
            };
            let list = shared.mcp.list(); // Non-blocking

            let table = lua.create_table()?;
            for (i, status) in list.into_iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("name", status.name)?;
                entry.set("state", status.state)?;
                entry.set("tools", status.tool_count)?;
                entry.set("endpoint", status.endpoint)?;
                entry.set("calls", status.call_count)?;
                if let Some(err) = status.error {
                    entry.set("error", err)?;
                }
                if let Some(attempt) = status.attempt {
                    entry.set("attempt", attempt)?;
                }
                if let Some(last_tool) = status.last_tool {
                    entry.set("last_tool", last_tool)?;
                }
                table.set(i + 1, entry)?;
            }
            Ok(table)
        })?
    };
    tools.set("mcp_list", mcp_list_fn)?;

    // =========================================================================
    // Telemetry Tools
    // =========================================================================

    // tools.trace_attr(key, value) -> nil
    // Add an attribute to the current tracing span
    // Note: tracing spans require fields to be declared at creation time,
    // so this logs the attribute as an event instead
    let trace_attr_fn = lua.create_function(|_, (key, value): (String, Value)| {
        match value {
            Value::String(s) => {
                if let Ok(s) = s.to_str() {
                    tracing::trace!(lua_attr_key = %key, lua_attr_value = %s);
                }
            }
            Value::Integer(i) => {
                tracing::trace!(lua_attr_key = %key, lua_attr_value = i);
            }
            Value::Number(n) => {
                tracing::trace!(lua_attr_key = %key, lua_attr_value = n);
            }
            Value::Boolean(b) => {
                tracing::trace!(lua_attr_key = %key, lua_attr_value = b);
            }
            _ => {} // Ignore complex types (tables, functions, etc.)
        }
        Ok(())
    })?;
    tools.set("trace_attr", trace_attr_fn)?;

    // tools.trace_attrs(table) -> nil
    // Add multiple attributes as trace events
    let trace_attrs_fn = lua.create_function(|_, attrs: Value| {
        if let Value::Table(t) = attrs {
            // Collect all attributes and log as a single event
            let json = lua_to_json(&Value::Table(t)).unwrap_or_default();
            tracing::trace!(lua_attrs = %json);
        }
        Ok(())
    })?;
    tools.set("trace_attrs", trace_attrs_fn)?;

    // tools.log_info(msg, fields) -> nil
    let log_info_fn = lua.create_function(|_, (msg, fields): (String, Option<Value>)| {
        match fields {
            Some(Value::Table(t)) => {
                let json = lua_to_json(&Value::Table(t)).unwrap_or_default();
                tracing::info!(message = %msg, lua_fields = %json);
            }
            _ => {
                tracing::info!(message = %msg);
            }
        }
        Ok(())
    })?;
    tools.set("log_info", log_info_fn)?;

    // tools.log_warn(msg, fields) -> nil
    let log_warn_fn = lua.create_function(|_, (msg, fields): (String, Option<Value>)| {
        match fields {
            Some(Value::Table(t)) => {
                let json = lua_to_json(&Value::Table(t)).unwrap_or_default();
                tracing::warn!(message = %msg, lua_fields = %json);
            }
            _ => {
                tracing::warn!(message = %msg);
            }
        }
        Ok(())
    })?;
    tools.set("log_warn", log_warn_fn)?;

    // tools.log_error(msg, fields) -> nil
    let log_error_fn = lua.create_function(|_, (msg, fields): (String, Option<Value>)| {
        match fields {
            Some(Value::Table(t)) => {
                let json = lua_to_json(&Value::Table(t)).unwrap_or_default();
                tracing::error!(message = %msg, lua_fields = %json);
            }
            _ => {
                tracing::error!(message = %msg);
            }
        }
        Ok(())
    })?;
    tools.set("log_error", log_error_fn)?;

    // tools.log_debug(msg, fields) -> nil
    let log_debug_fn = lua.create_function(|_, (msg, fields): (String, Option<Value>)| {
        match fields {
            Some(Value::Table(t)) => {
                let json = lua_to_json(&Value::Table(t)).unwrap_or_default();
                tracing::debug!(message = %msg, lua_fields = %json);
            }
            _ => {
                tracing::debug!(message = %msg);
            }
        }
        Ok(())
    })?;
    tools.set("log_debug", log_debug_fn)?;

    // tools.metric_counter(name, delta, labels) -> nil
    // Increment a counter metric with auto-context (room) and optional extra labels
    let metric_counter_fn = {
        let state = state.clone();
        lua.create_function(
            move |_, (name, delta, extra_labels): (String, Option<i64>, Option<Value>)| {
                let delta = delta.unwrap_or(1) as u64;

                // Auto-context from session context
                let room = state
                    .session_context()
                    .and_then(|ctx| ctx.room_name)
                    .unwrap_or_else(|| "lobby".to_string());

                // Build attributes
                let mut attrs = vec![opentelemetry::KeyValue::new("room", room)];

                // Add extra labels if provided
                if let Some(Value::Table(t)) = extra_labels {
                    for (k, v) in t.pairs::<String, Value>().flatten() {
                        let val = match v {
                            Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
                            Value::Integer(i) => Some(i.to_string()),
                            Value::Number(n) => Some(n.to_string()),
                            Value::Boolean(b) => Some(b.to_string()),
                            _ => None,
                        };
                        if let Some(val) = val {
                            attrs.push(opentelemetry::KeyValue::new(k, val));
                        }
                    }
                }

                // Record via OpenTelemetry metrics API
                let meter = opentelemetry::global::meter("sshwarma.lua");
                let counter = meter.u64_counter(name).build();
                counter.add(delta, &attrs);

                Ok(())
            },
        )?
    };
    tools.set("metric_counter", metric_counter_fn)?;

    // tools.metric_gauge(name, value, labels) -> nil
    // Set a gauge metric value with auto-context
    let metric_gauge_fn = {
        let state = state.clone();
        lua.create_function(
            move |_, (name, value, extra_labels): (String, f64, Option<Value>)| {
                // Auto-context from session context
                let room = state
                    .session_context()
                    .and_then(|ctx| ctx.room_name)
                    .unwrap_or_else(|| "lobby".to_string());

                // Build attributes
                let mut attrs = vec![opentelemetry::KeyValue::new("room", room)];

                // Add extra labels if provided
                if let Some(Value::Table(t)) = extra_labels {
                    for (k, v) in t.pairs::<String, Value>().flatten() {
                        let val = match v {
                            Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
                            Value::Integer(i) => Some(i.to_string()),
                            Value::Number(n) => Some(n.to_string()),
                            Value::Boolean(b) => Some(b.to_string()),
                            _ => None,
                        };
                        if let Some(val) = val {
                            attrs.push(opentelemetry::KeyValue::new(k, val));
                        }
                    }
                }

                // Record via OpenTelemetry metrics API (using observable gauge pattern)
                let meter = opentelemetry::global::meter("sshwarma.lua");
                let gauge = meter.f64_gauge(name).build();
                gauge.record(value, &attrs);

                Ok(())
            },
        )?
    };
    tools.set("metric_gauge", metric_gauge_fn)?;

    // tools.metric_histogram(name, value, labels) -> nil
    // Record a histogram observation with auto-context
    let metric_histogram_fn = {
        let state = state.clone();
        lua.create_function(
            move |_, (name, value, extra_labels): (String, f64, Option<Value>)| {
                // Auto-context from session context
                let room = state
                    .session_context()
                    .and_then(|ctx| ctx.room_name)
                    .unwrap_or_else(|| "lobby".to_string());

                // Build attributes
                let mut attrs = vec![opentelemetry::KeyValue::new("room", room)];

                // Add extra labels if provided
                if let Some(Value::Table(t)) = extra_labels {
                    for (k, v) in t.pairs::<String, Value>().flatten() {
                        let val = match v {
                            Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
                            Value::Integer(i) => Some(i.to_string()),
                            Value::Number(n) => Some(n.to_string()),
                            Value::Boolean(b) => Some(b.to_string()),
                            _ => None,
                        };
                        if let Some(val) = val {
                            attrs.push(opentelemetry::KeyValue::new(k, val));
                        }
                    }
                }

                // Record via OpenTelemetry metrics API
                let meter = opentelemetry::global::meter("sshwarma.lua");
                let histogram = meter.f64_histogram(name).build();
                histogram.record(value, &attrs);

                Ok(())
            },
        )?
    };
    tools.set("metric_histogram", metric_histogram_fn)?;

    // =========================================================================
    // Things system callbacks (inventory)
    // =========================================================================

    // things_get(qualified_name) -> thing table or nil
    let things_get_fn = {
        let state = state.clone();
        lua.create_function(move |lua, qualified_name: String| {
            let shared = match state.shared_state() {
                Some(s) => s,
                None => return Ok(Value::Nil),
            };

            match shared.db.get_thing_by_qualified_name(&qualified_name) {
                Ok(Some(thing)) => {
                    let t = lua.create_table()?;
                    t.set("id", thing.id)?;
                    t.set("parent_id", thing.parent_id)?;
                    t.set("kind", thing.kind.as_str())?;
                    t.set("name", thing.name)?;
                    t.set("qualified_name", thing.qualified_name)?;
                    t.set("description", thing.description)?;
                    t.set("content", thing.content)?;
                    t.set("available", thing.available)?;
                    Ok(Value::Table(t))
                }
                _ => Ok(Value::Nil),
            }
        })?
    };
    tools.set("things_get", things_get_fn)?;

    // things_children(parent_id) -> array of thing tables
    let things_children_fn = {
        let state = state.clone();
        lua.create_function(move |lua, parent_id: String| {
            let shared = match state.shared_state() {
                Some(s) => s,
                None => return lua.create_table(),
            };

            let children = shared.db.get_thing_children(&parent_id).unwrap_or_default();
            let result = lua.create_table()?;
            for (i, thing) in children.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("id", thing.id.clone())?;
                t.set("kind", thing.kind.as_str())?;
                t.set("name", thing.name.clone())?;
                t.set("qualified_name", thing.qualified_name.clone())?;
                t.set("available", thing.available)?;
                result.set(i + 1, t)?;
            }
            Ok(result)
        })?
    };
    tools.set("things_children", things_children_fn)?;

    // things_find(pattern) -> array of thing tables (glob search on qualified_name)
    let things_find_fn = {
        let state = state.clone();
        lua.create_function(move |lua, pattern: String| {
            let shared = match state.shared_state() {
                Some(s) => s,
                None => return lua.create_table(),
            };

            let things = shared
                .db
                .find_things_by_qualified_name(&pattern)
                .unwrap_or_default();
            let result = lua.create_table()?;
            for (i, thing) in things.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("id", thing.id.clone())?;
                t.set("kind", thing.kind.as_str())?;
                t.set("name", thing.name.clone())?;
                t.set("qualified_name", thing.qualified_name.clone())?;
                t.set("available", thing.available)?;
                result.set(i + 1, t)?;
            }
            Ok(result)
        })?
    };
    tools.set("things_find", things_find_fn)?;

    // things_by_kind(kind) -> array of thing tables
    let things_by_kind_fn = {
        let state = state.clone();
        lua.create_function(move |lua, kind_str: String| {
            use crate::db::things::ThingKind;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => return lua.create_table(),
            };

            let kind = match ThingKind::parse(&kind_str) {
                Some(k) => k,
                None => return lua.create_table(),
            };

            let things = shared.db.list_things_by_kind(kind).unwrap_or_default();
            let result = lua.create_table()?;
            for (i, thing) in things.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("id", thing.id.clone())?;
                t.set("kind", thing.kind.as_str())?;
                t.set("name", thing.name.clone())?;
                t.set("qualified_name", thing.qualified_name.clone())?;
                t.set("available", thing.available)?;
                result.set(i + 1, t)?;
            }
            Ok(result)
        })?
    };
    tools.set("things_by_kind", things_by_kind_fn)?;

    // equipped_list(context_id) -> array of equipped thing tables
    let equipped_list_fn = {
        let state = state.clone();
        lua.create_function(move |lua, context_id: String| {
            let shared = match state.shared_state() {
                Some(s) => s,
                None => return lua.create_table(),
            };

            let equipped = shared.db.get_equipped(&context_id).unwrap_or_default();
            let result = lua.create_table()?;
            for (i, eq) in equipped.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("context_id", eq.context_id.clone())?;
                t.set("priority", eq.priority)?;
                t.set("id", eq.thing.id.clone())?;
                t.set("kind", eq.thing.kind.as_str())?;
                t.set("name", eq.thing.name.clone())?;
                t.set("qualified_name", eq.thing.qualified_name.clone())?;
                t.set("available", eq.thing.available)?;
                result.set(i + 1, t)?;
            }
            Ok(result)
        })?
    };
    tools.set("equipped_list", equipped_list_fn)?;

    // equipped_tools(context_id) -> array of equipped tool tables (only available tools)
    let equipped_tools_fn = {
        let state = state.clone();
        lua.create_function(move |lua, context_id: String| {
            let shared = match state.shared_state() {
                Some(s) => s,
                None => return lua.create_table(),
            };

            let tools_list = shared
                .db
                .get_equipped_tools(&context_id)
                .unwrap_or_default();
            let result = lua.create_table()?;
            for (i, eq) in tools_list.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("id", eq.thing.id.clone())?;
                t.set("name", eq.thing.name.clone())?;
                t.set("qualified_name", eq.thing.qualified_name.clone())?;
                t.set("description", eq.thing.description.clone())?;
                t.set("priority", eq.priority)?;
                result.set(i + 1, t)?;
            }
            Ok(result)
        })?
    };
    tools.set("equipped_tools", equipped_tools_fn)?;

    // equipped_merged_tools(room_id, agent_id) -> merged tools from room + agent
    let equipped_merged_fn = {
        let state = state.clone();
        lua.create_function(move |lua, (room_id, agent_id): (String, String)| {
            let shared = match state.shared_state() {
                Some(s) => s,
                None => return lua.create_table(),
            };

            let tools_list = shared
                .db
                .get_merged_equipped_tools(&room_id, &agent_id)
                .unwrap_or_default();
            let result = lua.create_table()?;
            for (i, eq) in tools_list.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("id", eq.thing.id.clone())?;
                t.set("name", eq.thing.name.clone())?;
                t.set("qualified_name", eq.thing.qualified_name.clone())?;
                t.set("description", eq.thing.description.clone())?;
                t.set("context_id", eq.context_id.clone())?;
                t.set("priority", eq.priority)?;
                result.set(i + 1, t)?;
            }
            Ok(result)
        })?
    };
    tools.set("equipped_merged_tools", equipped_merged_fn)?;

    // equip(context_id, thing_id, priority) -> bool success
    let equip_fn = {
        let state = state.clone();
        lua.create_function(
            move |_lua, (context_id, thing_id, priority): (String, String, Option<f64>)| {
                let shared = match state.shared_state() {
                    Some(s) => s,
                    None => return Ok(false),
                };

                shared
                    .db
                    .equip(&context_id, &thing_id, priority.unwrap_or(0.0))
                    .is_ok()
                    .then_some(true)
                    .ok_or_else(|| mlua::Error::external("equip failed"))
            },
        )?
    };
    tools.set("equip", equip_fn)?;

    // unequip(context_id, thing_id) -> bool success
    let unequip_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, (context_id, thing_id): (String, String)| {
            let shared = match state.shared_state() {
                Some(s) => s,
                None => return Ok(false),
            };

            shared
                .db
                .unequip(&context_id, &thing_id)
                .is_ok()
                .then_some(true)
                .ok_or_else(|| mlua::Error::external("unequip failed"))
        })?
    };
    tools.set("unequip", unequip_fn)?;

    // exits_list(room_thing_id) -> array of {direction, target_name, target_id}
    let exits_list_fn = {
        let state = state.clone();
        lua.create_function(move |lua, room_thing_id: String| {
            let shared = match state.shared_state() {
                Some(s) => s,
                None => return lua.create_table(),
            };

            let exits = shared.db.get_exits_from(&room_thing_id).unwrap_or_default();
            let result = lua.create_table()?;
            for (i, exit) in exits.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("direction", exit.direction.clone())?;
                t.set("target_name", exit.target.name.clone())?;
                t.set("target_id", exit.target.id.clone())?;
                result.set(i + 1, t)?;
            }
            Ok(result)
        })?
    };
    tools.set("exits_list", exits_list_fn)?;

    // =========================================================================
    // Command Operations (structured data, not formatted strings)
    // =========================================================================

    // tools.join(room_name) -> {success, room, error}
    let join_fn = {
        let state = state.clone();
        lua.create_function(move |lua, room_name: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            // Use ops::join
            match tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(crate::ops::join(
                    &shared,
                    &session.username,
                    session.room_name.as_deref(),
                    &room_name,
                ))
            }) {
                Ok(room_summary) => {
                    // Update session context with new room
                    state.set_session_context(Some(crate::lua::SessionContext {
                        username: session.username.clone(),
                        model: None,
                        room_name: Some(room_name.clone()),
                    }));

                    result.set("success", true)?;
                    let room_table = lua.create_table()?;
                    room_table.set("name", room_summary.name)?;
                    if let Some(desc) = room_summary.description {
                        room_table.set("description", desc)?;
                    }
                    if let Some(vibe) = room_summary.vibe {
                        room_table.set("vibe", vibe)?;
                    }
                    let users_table = lua.create_table()?;
                    for (i, user) in room_summary.users.iter().enumerate() {
                        users_table.set(i + 1, user.clone())?;
                    }
                    room_table.set("users", users_table)?;
                    let models_table = lua.create_table()?;
                    for (i, model) in room_summary.models.iter().enumerate() {
                        models_table.set(i + 1, model.clone())?;
                    }
                    room_table.set("models", models_table)?;
                    result.set("room", room_table)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("join", join_fn)?;

    // tools.create(room_name, description?) -> {success, room, error}
    let create_fn = {
        let state = state.clone();
        lua.create_function(
            move |lua, (room_name, _description): (String, Option<String>)| {
                let result = lua.create_table()?;

                let shared = match state.shared_state() {
                    Some(s) => s,
                    None => {
                        result.set("success", false)?;
                        result.set("error", "no shared state")?;
                        return Ok(result);
                    }
                };

                let session = match state.session_context() {
                    Some(s) => s,
                    None => {
                        result.set("success", false)?;
                        result.set("error", "no session context")?;
                        return Ok(result);
                    }
                };

                // Use ops::create_room
                match tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(crate::ops::create_room(
                        &shared,
                        &session.username,
                        &room_name,
                        session.room_name.as_deref(),
                    ))
                }) {
                    Ok(room_summary) => {
                        // Update session context with new room (create auto-joins)
                        state.set_session_context(Some(crate::lua::SessionContext {
                            username: session.username.clone(),
                            model: None,
                            room_name: Some(room_name.clone()),
                        }));

                        result.set("success", true)?;
                        let room_table = lua.create_table()?;
                        room_table.set("name", room_summary.name)?;
                        result.set("room", room_table)?;
                    }
                    Err(e) => {
                        result.set("success", false)?;
                        result.set("error", e.to_string())?;
                    }
                }

                Ok(result)
            },
        )?
    };
    tools.set("create", create_fn)?;

    // tools.leave() -> {success, error}
    let leave_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            // Use ops::leave
            match tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(crate::ops::leave(
                    &shared,
                    &session.username,
                    &room_name,
                ))
            }) {
                Ok(()) => {
                    // Clear room from session context
                    state.set_session_context(Some(crate::lua::SessionContext {
                        username: session.username.clone(),
                        model: None,
                        room_name: None,
                    }));

                    result.set("success", true)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("leave", leave_fn)?;

    // tools.go(direction) -> {success, room, error}
    let go_fn = {
        let state = state.clone();
        lua.create_function(move |lua, direction: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            // Use ops::go
            match tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(crate::ops::go(
                    &shared,
                    &session.username,
                    &room_name,
                    &direction,
                ))
            }) {
                Ok(room_summary) => {
                    // Update session context with new room
                    state.set_session_context(Some(crate::lua::SessionContext {
                        username: session.username.clone(),
                        model: None,
                        room_name: Some(room_summary.name.clone()),
                    }));

                    result.set("success", true)?;
                    let room_table = lua.create_table()?;
                    room_table.set("name", room_summary.name)?;
                    if let Some(desc) = room_summary.description {
                        room_table.set("description", desc)?;
                    }
                    if let Some(vibe) = room_summary.vibe {
                        room_table.set("vibe", vibe)?;
                    }
                    result.set("room", room_table)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("go", go_fn)?;

    // tools.dig(direction, target_room, bidirectional?) -> {success, reverse, error}
    let dig_fn = {
        let state = state.clone();
        lua.create_function(
            move |lua, (direction, target_room, _bidirectional): (String, String, Option<bool>)| {
                let result = lua.create_table()?;

                let shared = match state.shared_state() {
                    Some(s) => s,
                    None => {
                        result.set("success", false)?;
                        result.set("error", "no shared state")?;
                        return Ok(result);
                    }
                };

                let session = match state.session_context() {
                    Some(s) => s,
                    None => {
                        result.set("success", false)?;
                        result.set("error", "no session context")?;
                        return Ok(result);
                    }
                };

                let room_name = match session.room_name {
                    Some(ref r) => r.clone(),
                    None => {
                        result.set("success", false)?;
                        result.set("error", "not in a room")?;
                        return Ok(result);
                    }
                };

                // Use ops::dig (always creates bidirectional)
                match tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(crate::ops::dig(
                        &shared,
                        &room_name,
                        &direction,
                        &target_room,
                    ))
                }) {
                    Ok(reverse) => {
                        result.set("success", true)?;
                        result.set("reverse", reverse)?;
                    }
                    Err(e) => {
                        result.set("success", false)?;
                        result.set("error", e.to_string())?;
                    }
                }

                Ok(result)
            },
        )?
    };
    tools.set("dig", dig_fn)?;

    // tools.fork(new_name) -> {success, room, error}
    let fork_fn = {
        let state = state.clone();
        lua.create_function(move |lua, new_name: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            // Use ops::fork_room
            match tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(crate::ops::fork_room(
                    &shared,
                    &session.username,
                    &room_name,
                    &new_name,
                ))
            }) {
                Ok(room_summary) => {
                    // Update session context with new room (fork auto-joins)
                    state.set_session_context(Some(crate::lua::SessionContext {
                        username: session.username.clone(),
                        model: None,
                        room_name: Some(new_name.clone()),
                    }));

                    result.set("success", true)?;
                    let room_table = lua.create_table()?;
                    room_table.set("name", room_summary.name)?;
                    if let Some(vibe) = room_summary.vibe {
                        room_table.set("vibe", vibe)?;
                    }
                    result.set("room", room_table)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("fork", fork_fn)?;

    // tools.inventory() -> {equipped = [...], available = [...]}
    let inventory_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    return Ok(result);
                }
            };

            // Get room thing ID if in a room
            let room_name = session.room_name.as_deref().unwrap_or("lobby");
            let context_id = room_name.to_string(); // Use room name as context_id

            // Get equipped tools
            let equipped_table = lua.create_table()?;
            if let Ok(equipped) = shared.db.get_equipped_tools(&context_id) {
                for (i, eq) in equipped.iter().enumerate() {
                    let t = lua.create_table()?;
                    t.set("id", eq.thing.id.clone())?;
                    t.set("name", eq.thing.name.clone())?;
                    t.set("qualified_name", eq.thing.qualified_name.clone())?;
                    t.set("description", eq.thing.description.clone())?;
                    t.set("priority", eq.priority)?;
                    equipped_table.set(i + 1, t)?;
                }
            }
            result.set("equipped", equipped_table)?;

            // Get available tools (all tools from things table)
            let available_table = lua.create_table()?;
            if let Ok(tools_list) = shared
                .db
                .list_things_by_kind(crate::db::things::ThingKind::Tool)
            {
                for (i, thing) in tools_list.iter().enumerate() {
                    if thing.available {
                        let t = lua.create_table()?;
                        t.set("id", thing.id.clone())?;
                        t.set("name", thing.name.clone())?;
                        t.set("qualified_name", thing.qualified_name.clone())?;
                        t.set("description", thing.description.clone())?;
                        available_table.set(i + 1, t)?;
                    }
                }
            }
            result.set("available", available_table)?;

            Ok(result)
        })?
    };
    tools.set("inventory", inventory_fn)?;

    // tools.equip_tool(qualified_name) -> {success, added, removed, equipped, error}
    let equip_tool_fn = {
        let state = state.clone();
        lua.create_function(move |lua, qualified_name: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let context_id = session.room_name.as_deref().unwrap_or("lobby").to_string();

            // Find the thing by qualified name
            let thing = match shared.db.get_thing_by_qualified_name(&qualified_name) {
                Ok(Some(t)) => t,
                Ok(None) => {
                    result.set("success", false)?;
                    result.set("error", format!("tool not found: {}", qualified_name))?;
                    return Ok(result);
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                    return Ok(result);
                }
            };

            // Get max priority and equip
            let priority = shared
                .db
                .max_equipped_priority(&context_id)
                .ok()
                .flatten()
                .map(|p| p + 1.0)
                .unwrap_or(0.0);

            match shared.db.equip(&context_id, &thing.id, priority) {
                Ok(()) => {
                    result.set("success", true)?;
                    let added_table = lua.create_table()?;
                    added_table.set(1, qualified_name)?;
                    result.set("added", added_table)?;
                    result.set("removed", lua.create_table()?)?;

                    // Return updated equipped list
                    let equipped_table = lua.create_table()?;
                    if let Ok(equipped) = shared.db.get_equipped_tools(&context_id) {
                        for (i, eq) in equipped.iter().enumerate() {
                            equipped_table.set(i + 1, eq.thing.qualified_name.clone())?;
                        }
                    }
                    result.set("equipped", equipped_table)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("equip_tool", equip_tool_fn)?;

    // tools.unequip_tool(qualified_name) -> {success, removed, equipped, error}
    let unequip_tool_fn = {
        let state = state.clone();
        lua.create_function(move |lua, qualified_name: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let context_id = session.room_name.as_deref().unwrap_or("lobby").to_string();

            // Find the thing by qualified name
            let thing = match shared.db.get_thing_by_qualified_name(&qualified_name) {
                Ok(Some(t)) => t,
                Ok(None) => {
                    result.set("success", false)?;
                    result.set("error", format!("tool not found: {}", qualified_name))?;
                    return Ok(result);
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                    return Ok(result);
                }
            };

            match shared.db.unequip(&context_id, &thing.id) {
                Ok(()) => {
                    result.set("success", true)?;
                    let removed_table = lua.create_table()?;
                    removed_table.set(1, qualified_name)?;
                    result.set("removed", removed_table)?;

                    // Return updated equipped list
                    let equipped_table = lua.create_table()?;
                    if let Ok(equipped) = shared.db.get_equipped_tools(&context_id) {
                        for (i, eq) in equipped.iter().enumerate() {
                            equipped_table.set(i + 1, eq.thing.qualified_name.clone())?;
                        }
                    }
                    result.set("equipped", equipped_table)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("unequip_tool", unequip_tool_fn)?;

    // tools.journal_add(kind, content) -> {success, entry, error}
    let journal_add_fn = {
        let state = state.clone();
        lua.create_function(move |lua, (kind, content): (String, String)| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            // Parse kind
            let ops_kind = match kind.as_str() {
                "note" => crate::ops::JournalKind::Note,
                "decision" => crate::ops::JournalKind::Decision,
                "idea" => crate::ops::JournalKind::Idea,
                "milestone" => crate::ops::JournalKind::Milestone,
                _ => {
                    result.set("success", false)?;
                    result.set("error", format!("invalid journal kind: {}", kind))?;
                    return Ok(result);
                }
            };

            // Use ops::add_journal
            match tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(crate::ops::add_journal(
                    &shared,
                    &room_name,
                    &session.username,
                    &content,
                    ops_kind,
                ))
            }) {
                Ok(()) => {
                    result.set("success", true)?;
                    let entry = lua.create_table()?;
                    entry.set("kind", kind)?;
                    entry.set("content", content)?;
                    entry.set("author", session.username)?;
                    result.set("entry", entry)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("journal_add", journal_add_fn)?;

    // tools.set_vibe(text) -> {success, error}
    let set_vibe_fn = {
        let state = state.clone();
        lua.create_function(move |lua, text: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            // Use ops::set_vibe
            match tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(crate::ops::set_vibe(&shared, &room_name, &text))
            }) {
                Ok(()) => {
                    result.set("success", true)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("set_vibe", set_vibe_fn)?;

    // tools.inspire(text?) -> {inspirations = [...]} or {success, added, error}
    let inspire_fn = {
        let state = state.clone();
        lua.create_function(move |lua, text: Option<String>| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    return Ok(result);
                }
            };

            match text {
                Some(content) => {
                    // Add inspiration
                    match tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(crate::ops::add_inspiration(
                            &shared,
                            &room_name,
                            &content,
                            &session.username,
                        ))
                    }) {
                        Ok(()) => {
                            result.set("success", true)?;
                            result.set("added", content)?;
                        }
                        Err(e) => {
                            result.set("success", false)?;
                            result.set("error", e.to_string())?;
                        }
                    }
                }
                None => {
                    // Get inspirations
                    let insps_table = lua.create_table()?;
                    if let Ok(insps) = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(crate::ops::get_inspirations(&shared, &room_name))
                    }) {
                        for (i, insp) in insps.iter().enumerate() {
                            insps_table.set(i + 1, insp.content.clone())?;
                        }
                    }
                    result.set("inspirations", insps_table)?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("inspire", inspire_fn)?;

    // tools.bring(artifact_id, role) -> {success, error}
    let bring_fn = {
        let state = state.clone();
        lua.create_function(move |lua, (artifact_id, role): (String, String)| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            // Use ops::bind_asset
            match tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(crate::ops::bind_asset(
                    &shared,
                    &room_name,
                    &role,
                    &artifact_id,
                    &session.username,
                ))
            }) {
                Ok(()) => {
                    result.set("success", true)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("bring", bring_fn)?;

    // tools.drop_asset(role) -> {success, error}
    let drop_asset_fn = {
        let state = state.clone();
        lua.create_function(move |lua, role: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            // Use ops::unbind_asset
            match tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(crate::ops::unbind_asset(&shared, &room_name, &role))
            }) {
                Ok(()) => {
                    result.set("success", true)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("drop_asset", drop_asset_fn)?;

    // tools.examine(role) -> {asset, error}
    let examine_fn = {
        let state = state.clone();
        lua.create_function(move |lua, role: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            // Use ops::examine_asset
            match tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(crate::ops::examine_asset(&shared, &room_name, &role))
            }) {
                Ok(Some(binding)) => {
                    let asset = lua.create_table()?;
                    asset.set("role", binding.role)?;
                    asset.set("artifact_id", binding.artifact_id)?;
                    if let Some(notes) = binding.notes {
                        asset.set("notes", notes)?;
                    }
                    asset.set("bound_by", binding.bound_by)?;
                    asset.set("bound_at", binding.bound_at)?;
                    result.set("asset", asset)?;
                }
                Ok(None) => {
                    // No binding found - result is just empty
                }
                Err(e) => {
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("examine", examine_fn)?;

    // tools.mcp_servers() -> {servers = [{name, connected, tool_count}, ...]}
    let mcp_servers_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let result = lua.create_table()?;
            let servers_table = lua.create_table()?;

            if let Some(shared) = state.shared_state() {
                for (i, server) in shared.mcp.list().iter().enumerate() {
                    let s = lua.create_table()?;
                    s.set("name", server.name.clone())?;
                    s.set("connected", server.state == "connected")?;
                    s.set("tool_count", server.tool_count)?;
                    s.set("endpoint", server.endpoint.clone())?;
                    if let Some(ref error) = server.error {
                        s.set("error", error.clone())?;
                    }
                    servers_table.set(i + 1, s)?;
                }
            }

            result.set("servers", servers_table)?;
            Ok(result)
        })?
    };
    tools.set("mcp_servers", mcp_servers_fn)?;

    // tools.mcp_tools(server?) -> {tools = [{name, description, server}, ...]}
    let mcp_tools_fn = {
        let state = state.clone();
        lua.create_function(move |lua, server_filter: Option<String>| {
            let result = lua.create_table()?;
            let tools_table = lua.create_table()?;

            if let Some(shared) = state.shared_state() {
                let tool_list = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(shared.mcp.list_tools())
                });

                let mut idx = 1;
                for tool in tool_list {
                    // Filter by server if specified
                    if let Some(ref filter) = server_filter {
                        if &tool.source != filter {
                            continue;
                        }
                    }

                    let t = lua.create_table()?;
                    t.set("name", tool.name)?;
                    t.set("description", tool.description)?;
                    t.set("server", tool.source)?;
                    tools_table.set(idx, t)?;
                    idx += 1;
                }
            }

            result.set("tools", tools_table)?;
            Ok(result)
        })?
    };
    tools.set("mcp_tools", mcp_tools_fn)?;

    // tools.prompts(target?) -> {prompts = [{name, content, priority}, ...]}
    let prompts_fn = {
        let state = state.clone();
        lua.create_function(move |lua, target: Option<String>| {
            let result = lua.create_table()?;
            let prompts_table = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("prompts", prompts_table)?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("prompts", prompts_table)?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("prompts", prompts_table)?;
                    return Ok(result);
                }
            };

            match target {
                Some(t) => {
                    // Get prompts for specific target
                    if let Ok(slots) = shared.db.get_target_slots(&room_name, &t) {
                        for (i, slot) in slots.iter().enumerate() {
                            let p = lua.create_table()?;
                            p.set("name", slot.prompt_name.clone())?;
                            p.set("priority", slot.index)?;
                            if let Some(ref content) = slot.content {
                                p.set("content", content.clone())?;
                            }
                            prompts_table.set(i + 1, p)?;
                        }
                    }
                }
                None => {
                    // List all prompts in room
                    if let Ok(prompts_list) = shared.db.list_prompts(&room_name) {
                        for (i, prompt) in prompts_list.iter().enumerate() {
                            let p = lua.create_table()?;
                            p.set("name", prompt.name.clone())?;
                            p.set("content", prompt.content.clone())?;
                            prompts_table.set(i + 1, p)?;
                        }
                    }
                }
            }

            result.set("prompts", prompts_table)?;
            Ok(result)
        })?
    };
    tools.set("prompts", prompts_fn)?;

    // tools.prompt_set(name, content) -> {success, error}
    let prompt_set_fn = {
        let state = state.clone();
        lua.create_function(move |lua, (name, content): (String, String)| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            match shared
                .db
                .set_prompt(&room_name, &name, &content, &session.username)
            {
                Ok(()) => {
                    result.set("success", true)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("prompt_set", prompt_set_fn)?;

    // tools.prompt_push(target, prompt_name) -> {success, error}
    let prompt_push_fn = {
        let state = state.clone();
        lua.create_function(move |lua, (target, prompt_name): (String, String)| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            match shared
                .db
                .push_slot(&room_name, &target, "user", &prompt_name)
            {
                Ok(()) => {
                    result.set("success", true)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("prompt_push", prompt_push_fn)?;

    // tools.prompt_pop(target) -> {success, removed, error}
    let prompt_pop_fn = {
        let state = state.clone();
        lua.create_function(move |lua, target: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            match shared.db.pop_slot(&room_name, &target) {
                Ok(removed) => {
                    result.set("success", true)?;
                    result.set("removed", removed)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("prompt_pop", prompt_pop_fn)?;

    // tools.prompt_delete(name) -> {success, error}
    let prompt_delete_fn = {
        let state = state.clone();
        lua.create_function(move |lua, name: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            match shared.db.delete_prompt(&room_name, &name) {
                Ok(deleted) => {
                    result.set("success", deleted)?;
                    if !deleted {
                        result.set("error", "prompt not found")?;
                    }
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("prompt_delete", prompt_delete_fn)?;

    // tools.prompt_rm(target, index) -> {success, error}
    // Remove prompt from target by slot index (0-based)
    let prompt_rm_fn = {
        let state = state.clone();
        lua.create_function(move |lua, (target, index): (String, i64)| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            match shared.db.rm_slot(&room_name, &target, index) {
                Ok(removed) => {
                    result.set("success", removed)?;
                    if !removed {
                        result.set("error", "slot not found")?;
                    }
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("prompt_rm", prompt_rm_fn)?;

    // tools.prompt_insert(target, index, prompt_name) -> {success, error}
    // Insert prompt into target at slot index
    let prompt_insert_fn = {
        let state = state.clone();
        lua.create_function(move |lua, (target, index, prompt_name): (String, i64, String)| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no session context")?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("success", false)?;
                    result.set("error", "not in a room")?;
                    return Ok(result);
                }
            };

            // Use "system" as default target_type for manual insertion
            match shared.db.insert_slot(&room_name, &target, "system", index, &prompt_name) {
                Ok(()) => {
                    result.set("success", true)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("prompt_insert", prompt_insert_fn)?;

    // tools.target_slots(target) -> [{slot, prompt_name}]
    // Get slots for a specific target
    let target_slots_fn = {
        let state = state.clone();
        lua.create_function(move |lua, target: String| {
            let list = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => return Ok(list),
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => return Ok(list),
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => return Ok(list),
            };

            if let Ok(slots) = shared.db.get_target_slots(&room_name, &target) {
                let mut idx = 1;
                for slot in slots {
                    let entry = lua.create_table()?;
                    entry.set("slot", slot.index)?;
                    entry.set("prompt_name", slot.prompt_name)?;
                    if let Some(ref content) = slot.content {
                        entry.set("content", content.as_str())?;
                    }
                    list.set(idx, entry)?;
                    idx += 1;
                }
            }

            Ok(list)
        })?
    };
    tools.set("target_slots", target_slots_fn)?;

    // tools.get_prompt(name) -> {name, content} or nil
    // Get a specific named prompt
    let get_prompt_fn = {
        let state = state.clone();
        lua.create_function(move |lua, name: String| {
            let shared = match state.shared_state() {
                Some(s) => s,
                None => return Ok(mlua::Value::Nil),
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => return Ok(mlua::Value::Nil),
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => return Ok(mlua::Value::Nil),
            };

            match shared.db.get_prompt(&room_name, &name) {
                Ok(Some(prompt)) => {
                    let result = lua.create_table()?;
                    result.set("name", prompt.name)?;
                    result.set("content", prompt.content)?;
                    Ok(mlua::Value::Table(result))
                }
                _ => Ok(mlua::Value::Nil),
            }
        })?
    };
    tools.set("get_prompt", get_prompt_fn)?;

    // tools.rules() -> {rules = [{id, name, trigger, script, enabled}, ...]}
    let rules_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let result = lua.create_table()?;
            let rules_table = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("rules", rules_table)?;
                    return Ok(result);
                }
            };

            let session = match state.session_context() {
                Some(s) => s,
                None => {
                    result.set("rules", rules_table)?;
                    return Ok(result);
                }
            };

            let room_name = match session.room_name {
                Some(ref r) => r.clone(),
                None => {
                    result.set("rules", rules_table)?;
                    return Ok(result);
                }
            };

            // Get room ID
            if let Ok(Some(room)) = shared.db.get_room_by_name(&room_name) {
                if let Ok(rules_list) = shared.db.list_room_rules(&room.id) {
                    for (i, rule) in rules_list.iter().enumerate() {
                        let r = lua.create_table()?;
                        r.set("id", rule.id.clone())?;
                        r.set("name", rule.name.clone())?;
                        r.set("trigger", rule.trigger_kind.as_str())?;
                        r.set("script_id", rule.script_id.clone())?;
                        r.set("enabled", rule.enabled)?;
                        r.set("priority", rule.priority)?;
                        rules_table.set(i + 1, r)?;
                    }
                }
            }

            result.set("rules", rules_table)?;
            Ok(result)
        })?
    };
    tools.set("rules", rules_fn)?;

    // tools.rules_add(trigger_kind, script_name, opts) -> {success, rule_id, error}
    let rules_add_fn = {
        let state = state.clone();
        lua.create_function(
            move |lua, (trigger_kind, script_name, opts): (String, String, Option<Table>)| {
                use crate::db::rules::{ActionSlot, RoomRule, TriggerKind};

                let result = lua.create_table()?;

                let shared = match state.shared_state() {
                    Some(s) => s,
                    None => {
                        result.set("success", false)?;
                        result.set("error", "no shared state")?;
                        return Ok(result);
                    }
                };

                let session = match state.session_context() {
                    Some(s) => s,
                    None => {
                        result.set("success", false)?;
                        result.set("error", "no session context")?;
                        return Ok(result);
                    }
                };

                let room_name = match session.room_name {
                    Some(ref r) => r.clone(),
                    None => {
                        result.set("success", false)?;
                        result.set("error", "not in a room")?;
                        return Ok(result);
                    }
                };

                // Get room ID
                let room = match shared.db.get_room_by_name(&room_name) {
                    Ok(Some(r)) => r,
                    _ => {
                        result.set("success", false)?;
                        result.set("error", "room not found")?;
                        return Ok(result);
                    }
                };

                // Get script by name
                let script = match shared.db.get_script_by_name(&script_name) {
                    Ok(Some(s)) => s,
                    _ => {
                        result.set("success", false)?;
                        result.set("error", format!("script not found: {}", script_name))?;
                        return Ok(result);
                    }
                };

                // Parse trigger kind
                let kind = match TriggerKind::parse(&trigger_kind) {
                    Some(k) => k,
                    None => {
                        result.set("success", false)?;
                        result.set("error", format!("invalid trigger kind: {}", trigger_kind))?;
                        return Ok(result);
                    }
                };

                // Create rule based on kind
                let mut rule = match kind {
                    TriggerKind::Row => {
                        RoomRule::row_trigger(&room.id, &script.id, ActionSlot::Background)
                    }
                    TriggerKind::Tick => {
                        let divisor: i32 = opts
                            .as_ref()
                            .and_then(|t| t.get::<i32>("tick_divisor").ok())
                            .unwrap_or(1);
                        RoomRule::tick_trigger(&room.id, &script.id, divisor)
                    }
                    TriggerKind::Interval => {
                        let interval: i64 = opts
                            .as_ref()
                            .and_then(|t| t.get::<i64>("interval_ms").ok())
                            .unwrap_or(1000);
                        RoomRule::interval_trigger(&room.id, &script.id, interval)
                    }
                };

                // Apply optional name
                if let Some(ref t) = opts {
                    if let Ok(name) = t.get::<String>("name") {
                        rule.name = Some(name);
                    }
                }

                match shared.db.insert_rule(&rule) {
                    Ok(()) => {
                        result.set("success", true)?;
                        result.set("rule_id", rule.id)?;
                    }
                    Err(e) => {
                        result.set("success", false)?;
                        result.set("error", e.to_string())?;
                    }
                }

                Ok(result)
            },
        )?
    };
    tools.set("rules_add", rules_add_fn)?;

    // tools.rules_del(rule_id) -> {success, error}
    let rules_del_fn = {
        let state = state.clone();
        lua.create_function(move |lua, rule_id: String| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            match shared.db.delete_rule(&rule_id) {
                Ok(()) => {
                    result.set("success", true)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("rules_del", rules_del_fn)?;

    // tools.rules_enable(rule_id, enabled) -> {success, error}
    let rules_enable_fn = {
        let state = state.clone();
        lua.create_function(move |lua, (rule_id, enabled): (String, bool)| {
            let result = lua.create_table()?;

            let shared = match state.shared_state() {
                Some(s) => s,
                None => {
                    result.set("success", false)?;
                    result.set("error", "no shared state")?;
                    return Ok(result);
                }
            };

            match shared.db.set_rule_enabled(&rule_id, enabled) {
                Ok(()) => {
                    result.set("success", true)?;
                }
                Err(e) => {
                    result.set("success", false)?;
                    result.set("error", e.to_string())?;
                }
            }

            Ok(result)
        })?
    };
    tools.set("rules_enable", rules_enable_fn)?;

    // tools.scripts() -> {scripts = [{id, name, kind, description}, ...]}
    let scripts_fn = {
        let state = state.clone();
        lua.create_function(move |lua, ()| {
            let result = lua.create_table()?;
            let scripts_table = lua.create_table()?;

            if let Some(shared) = state.shared_state() {
                if let Ok(scripts_list) = shared.db.list_scripts(None) {
                    for (i, script) in scripts_list.iter().enumerate() {
                        let s = lua.create_table()?;
                        s.set("id", script.id.clone())?;
                        s.set("name", script.name.clone())?;
                        s.set("kind", script.kind.as_str())?;
                        s.set("description", script.description.clone())?;
                        scripts_table.set(i + 1, s)?;
                    }
                }
            }

            result.set("scripts", scripts_table)?;
            Ok(result)
        })?
    };
    tools.set("scripts", scripts_fn)?;

    // bootstrap_world() -> bool success (ensure world structure exists)
    let bootstrap_world_fn = {
        let state = state.clone();
        lua.create_function(move |_lua, ()| {
            let shared = match state.shared_state() {
                Some(s) => s,
                None => return Ok(false),
            };

            Ok(shared.db.bootstrap_world().is_ok())
        })?
    };
    tools.set("bootstrap_world", bootstrap_world_fn)?;

    // Register tool middleware functions
    crate::lua::tool_middleware::register_middleware_tools(lua, &tools, state.middleware.clone())?;

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

/// Register the unified `sshwarma.call(name, args)` interface
///
/// This provides a single entry point for all tool calls:
/// - Lua handlers take priority (registered via sshwarma.register)
/// - Builtins (status, time, room, etc.) are next
/// - MCP tools are fallback
pub fn register_sshwarma_call(lua: &Lua, state: LuaToolState) -> LuaResult<()> {
    use crate::lua::registry::{ToolContext, ToolRegistry};

    let sshwarma = lua.create_table()?;

    // Store Lua handlers in a table
    let lua_handlers: Table = lua.create_table()?;
    lua.set_named_registry_value("sshwarma_handlers", lua_handlers)?;

    // Create the tool registry
    let registry = std::sync::Arc::new(ToolRegistry::new());

    // sshwarma.call(name, args) -> result
    let call_fn = {
        let state = state.clone();
        let registry = registry.clone();
        lua.create_function(move |lua, (name, args): (String, Value)| {
            // 1. Check for Lua handler first
            let handlers: Table = lua.named_registry_value("sshwarma_handlers")?;
            if let Ok(handler) = handlers.get::<mlua::Function>(name.as_str()) {
                // Call Lua handler
                return handler.call::<Value>(args);
            }

            // 2. Try builtin tool
            if registry.has(&name) {
                // Build context from current state
                let shared = state.shared_state();
                let session = state.session_context();
                let status_tracker = state.status_tracker().clone();

                let ctx = if let Some(ref shared) = shared {
                    let mut ctx = ToolContext::new(shared, status_tracker);
                    if let Some(ref sess) = session {
                        ctx = ctx.with_user(sess.username.clone());
                        if let Some(ref room) = sess.room_name {
                            ctx = ctx.with_room(room.clone());
                        }
                    }
                    ctx
                } else {
                    // Minimal context without SharedState - create empty world
                    ToolContext {
                        db: std::sync::Arc::new(
                            crate::db::Database::in_memory()
                                .map_err(|e| mlua::Error::external(format!("db error: {}", e)))?,
                        ),
                        mcp: std::sync::Arc::new(crate::mcp::McpManager::new()),
                        world: std::sync::Arc::new(tokio::sync::RwLock::new(
                            crate::world::World::new(),
                        )),
                        status_tracker,
                        username: session.as_ref().map(|s| s.username.clone()),
                        room: session.as_ref().and_then(|s| s.room_name.clone()),
                    }
                };

                let json_args = lua_to_json(&args)?;
                match registry.call(&name, &ctx, json_args) {
                    Ok(result) => return json_to_lua(lua, &result),
                    Err(e) => {
                        return Err(mlua::Error::external(format!("tool error: {}", e)));
                    }
                }
            }

            // 3. Tool not found
            Err(mlua::Error::external(format!("unknown tool: {}", name)))
        })?
    };
    sshwarma.set("call", call_fn)?;

    // sshwarma.register(name, handler) - register Lua handler
    let register_fn = lua.create_function(|lua, (name, handler): (String, mlua::Function)| {
        let handlers: Table = lua.named_registry_value("sshwarma_handlers")?;
        handlers.set(name, handler)?;
        Ok(())
    })?;
    sshwarma.set("register", register_fn)?;

    // sshwarma.list() - list available tools
    let list_fn = {
        let registry = registry.clone();
        lua.create_function(move |lua, ()| {
            let builtins = registry.list();
            let table = lua.create_table()?;
            for (i, name) in builtins.iter().enumerate() {
                table.set(i + 1, name.as_str())?;
            }
            Ok(table)
        })?
    };
    sshwarma.set("list", list_fn)?;

    // =========================================
    // UI Primitives (return UserData, not JSON)
    // =========================================

    // sshwarma.screen(width, height) -> LuaArea
    // Creates an area representing the full terminal
    let screen_fn = lua.create_function(|_lua, (width, height): (u16, u16)| {
        Ok(LuaArea {
            rect: Rect::full(width, height),
            name: "screen".to_string(),
        })
    })?;
    sshwarma.set("screen", screen_fn)?;

    // sshwarma.buffer(width, height) -> LuaDrawContext
    // Creates a new render buffer and returns a draw context for the full area
    let buffer_fn = lua.create_function(|_lua, (width, height): (u16, u16)| {
        let buffer = Arc::new(std::sync::Mutex::new(RenderBuffer::new(width, height)));
        Ok(LuaDrawContext::new(buffer, 0, 0, width, height))
    })?;
    sshwarma.set("buffer", buffer_fn)?;

    // sshwarma.area(x, y, width, height) -> LuaArea
    // Creates an area with explicit bounds
    let area_fn = lua.create_function(|_lua, (x, y, w, h): (u16, u16, u16, u16)| {
        Ok(LuaArea {
            rect: Rect::new(x, y, w, h),
            name: "area".to_string(),
        })
    })?;
    sshwarma.set("area", area_fn)?;

    lua.globals().set("sshwarma", sshwarma)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::db::Database;
    use crate::llm::LlmClient;
    use crate::mcp::McpManager;
    use crate::model::{ModelBackend, ModelRegistry};
    use crate::rules::RulesEngine;
    use crate::state::SharedState;
    use crate::world::{Inspiration, JournalEntry, JournalKind, World};
    use chrono::Utc;
    use mlua::Function;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// Reusable test fixture with real components (no mocks)
    struct TestInstance {
        shared_state: Arc<SharedState>,
        #[allow(dead_code)]
        db: Arc<Database>,
        world: Arc<RwLock<World>>,
        models: Arc<ModelRegistry>,
    }

    impl TestInstance {
        fn new() -> anyhow::Result<Self> {
            let db = Arc::new(Database::open(":memory:")?);

            let mut models = ModelRegistry::new();
            models.register(ModelHandle {
                short_name: "test".to_string(),
                display_name: "Test Model".to_string(),
                backend: ModelBackend::Mock {
                    prefix: "[test]".to_string(),
                },
                available: true,
                system_prompt: Some("You are a test assistant.".to_string()),
                context_window: Some(8000),
            });
            let models = Arc::new(models);

            let world = Arc::new(RwLock::new(World::new()));

            let shared_state = Arc::new(SharedState {
                world: world.clone(),
                db: db.clone(),
                config: Config::default(),
                llm: Arc::new(LlmClient::new()?),
                models: models.clone(),
                mcp: Arc::new(McpManager::new()),
                rules: Arc::new(RulesEngine::new()),
            });

            Ok(Self {
                shared_state,
                db,
                world,
                models,
            })
        }

        /// Create room with optional vibe
        async fn create_room(&self, name: &str, vibe: Option<&str>) {
            let mut world = self.world.write().await;
            world.create_room(name.to_string());
            if let Some(v) = vibe {
                if let Some(room) = world.get_room_mut(name) {
                    room.context.vibe = Some(v.to_string());
                }
            }
        }

        /// Add chat message to room's buffer
        async fn add_message(&self, room: &str, sender: &str, content: &str) {
            use crate::db::rows::Row;

            // Get or create buffer
            if let Ok(buffer) = self.db.get_or_create_room_buffer(room) {
                // Get or create agent
                if let Ok(agent) = self.db.get_or_create_human_agent(sender) {
                    let mut row = Row::message(&buffer.id, &agent.id, content, false);
                    let _ = self.db.append_row(&mut row);
                }
            }
        }

        /// Add a thinking/streaming row (simulates model response in progress)
        async fn add_thinking(
            &self,
            room: &str,
            model_name: &str,
            content: &str,
        ) -> Option<String> {
            use crate::db::rows::Row;

            let buffer = self.db.get_or_create_room_buffer(room).ok()?;
            let agent = self.db.get_or_create_model_agent(model_name).ok()?;
            let mut row = Row::thinking(&buffer.id, &agent.id);
            row.content = Some(content.to_string());
            self.db.append_row(&mut row).ok()?;
            Some(row.id)
        }

        /// Finalize a thinking row (simulates streaming complete)
        fn finalize_thinking(&self, row_id: &str) {
            let _ = self.db.finalize_row(row_id);
        }

        /// Add journal entry
        async fn add_journal(&self, room: &str, kind: JournalKind, content: &str) {
            let mut world = self.world.write().await;
            if let Some(r) = world.get_room_mut(room) {
                r.context.journal.push(JournalEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    author: "test".to_string(),
                    content: content.to_string(),
                    kind,
                });
            }
        }

        /// Add inspiration
        async fn add_inspiration(&self, room: &str, content: &str) {
            let mut world = self.world.write().await;
            if let Some(r) = world.get_room_mut(room) {
                r.context.inspirations.push(Inspiration {
                    id: uuid::Uuid::new_v4().to_string(),
                    content: content.to_string(),
                    added_by: "test".to_string(),
                    added_at: Utc::now(),
                });
            }
        }

        /// Get a SessionContext for this instance
        fn session_context(&self, room: &str) -> SessionContext {
            let model = self.models.get("test").cloned();
            SessionContext {
                username: "testuser".to_string(),
                model,
                room_name: Some(room.to_string()),
            }
        }

        /// Create a configured LuaToolState with shared_state and session_context
        fn lua_tool_state(&self, room: &str) -> LuaToolState {
            let state = LuaToolState::new();
            state.set_shared_state(Some(self.shared_state.clone()));
            state.set_session_context(Some(self.session_context(room)));
            state
        }
    }

    #[test]
    fn test_register_tools() {
        let lua = Lua::new();
        let state = LuaToolState::new();

        register_tools(&lua, state).expect("should register tools");

        // Verify tools table exists
        let tools: Table = lua.globals().get("tools").expect("should have tools");

        // Verify functions exist
        let _look: Function = tools.get("look").expect("should have look");
        let _who: Function = tools.get("who").expect("should have who");
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
        lua.load(
            r#"
            tools.kv_set("test.key", {foo = "bar", count = 42})
            local val = tools.kv_get("test.key")
            assert(val.foo == "bar", "foo should be bar")
            assert(val.count == 42, "count should be 42")
        "#,
        )
        .exec()
        .expect("kv_set/kv_get should work");

        // Test kv_delete
        lua.load(
            r#"
            local old = tools.kv_delete("test.key")
            assert(old.foo == "bar", "deleted value should have foo")
            local gone = tools.kv_get("test.key")
            assert(gone == nil, "key should be deleted")
        "#,
        )
        .exec()
        .expect("kv_delete should work");
    }

    // =========================================================================
    // Phase 2: Utility Tool Tests
    // =========================================================================

    #[test]
    fn test_display_width() {
        let lua = Lua::new();
        let state = LuaToolState::new();
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            -- ASCII: 1 cell per char
            assert(tools.display_width("hello") == 5, "ASCII width")

            -- CJK: 2 cells per char (東京 = 2 chars, 4 cells)
            assert(tools.display_width("東京") == 4, "CJK width should be 4, got " .. tools.display_width("東京"))

            -- Mixed: "Hi東京" = 2 + 4 = 6 cells
            assert(tools.display_width("Hi東京") == 6, "mixed width")

            -- Emoji: typically 2 cells (implementation may vary)
            local emoji_width = tools.display_width("🎵")
            assert(emoji_width >= 1 and emoji_width <= 2, "emoji width should be 1-2")

            -- Empty string
            assert(tools.display_width("") == 0, "empty string")
        "#,
        )
        .exec()
        .expect("display_width should work");
    }

    #[test]
    fn test_estimate_tokens() {
        let lua = Lua::new();
        let state = LuaToolState::new();
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local tokens = tools.estimate_tokens("hello world test!")
            -- 17 chars / 4 = 4 tokens
            assert(tokens == 4, "expected 4 tokens, got " .. tostring(tokens))
        "#,
        )
        .exec()
        .expect("estimate_tokens should work");
    }

    #[test]
    fn test_truncate_short_text() {
        let lua = Lua::new();
        let state = LuaToolState::new();
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local short = tools.truncate("hello world", 100)
            assert(short == "hello world", "short text should be unchanged")
        "#,
        )
        .exec()
        .expect("truncate short text should work");
    }

    #[test]
    fn test_truncate_long_text() {
        let lua = Lua::new();
        let state = LuaToolState::new();
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            -- Create text that exceeds 2 tokens (8 chars)
            local long = "a b c d e f g h i j k l m n o p"
            local truncated = tools.truncate(long, 2)
            assert(string.find(truncated, "%.%.%."), "long text should have ellipsis")
            assert(#truncated < #long, "truncated should be shorter")
        "#,
        )
        .exec()
        .expect("truncate long text should work");
    }

    // =========================================================================
    // Phase 3: Session Context Tool Tests
    // =========================================================================

    #[test]
    fn test_current_user_without_context() {
        let lua = Lua::new();
        let state = LuaToolState::new();
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local user = tools.current_user()
            assert(user == nil, "no session context should return nil")
        "#,
        )
        .exec()
        .expect("current_user without context should return nil");
    }

    #[test]
    fn test_current_model_without_context() {
        let lua = Lua::new();
        let state = LuaToolState::new();
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local model = tools.current_model()
            assert(model == nil, "no session context should return nil")
        "#,
        )
        .exec()
        .expect("current_model without context should return nil");
    }

    #[tokio::test]
    async fn test_current_user_with_context() {
        let instance = TestInstance::new().expect("should create instance");
        instance.create_room("testroom", None).await;

        let lua = Lua::new();
        let state = LuaToolState::new();
        state.set_shared_state(Some(instance.shared_state.clone()));
        state.set_session_context(Some(instance.session_context("testroom")));
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local user = tools.current_user()
            assert(user ~= nil, "should have user")
            assert(user.name == "testuser", "username should be testuser")
        "#,
        )
        .exec()
        .expect("current_user with context should work");
    }

    #[tokio::test]
    async fn test_current_model_with_context() {
        let instance = TestInstance::new().expect("should create instance");
        instance.create_room("testroom", None).await;

        let lua = Lua::new();
        let state = LuaToolState::new();
        state.set_shared_state(Some(instance.shared_state.clone()));
        state.set_session_context(Some(instance.session_context("testroom")));
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local model = tools.current_model()
            assert(model ~= nil, "should have model")
            assert(model.name == "test", "short_name should be test")
            assert(model.display == "Test Model", "display_name")
            assert(model.context_window == 8000, "context_window")
        "#,
        )
        .exec()
        .expect("current_model with context should work");
    }

    // =========================================================================
    // Phase 4: SharedState Data Tool Tests
    // =========================================================================

    #[test]
    fn test_history_without_shared_state() {
        let lua = Lua::new();
        let state = LuaToolState::new();
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local messages = tools.history(10)
            assert(#messages == 0, "no shared state should return empty list")
        "#,
        )
        .exec()
        .expect("history without shared state should return empty");
    }

    #[test]
    fn test_history_with_shared_state() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let instance = TestInstance::new().expect("should create instance");

        rt.block_on(async {
            instance.create_room("testroom", Some("A test vibe")).await;
            instance
                .add_message("testroom", "alice", "Hello world")
                .await;
            instance.add_message("testroom", "bob", "Hi alice!").await;
        });

        let lua = Lua::new();
        let state = instance.lua_tool_state("testroom");
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local messages = tools.history(10)
            assert(#messages == 2, "should have 2 messages, got " .. #messages)
            assert(messages[1].author == "alice", "first author should be alice")
            assert(messages[1].content == "Hello world", "first content")
            assert(messages[2].author == "bob", "second author should be bob")
        "#,
        )
        .exec()
        .expect("history with shared state should work");
    }

    #[test]
    fn test_journal_without_shared_state() {
        let lua = Lua::new();
        let state = LuaToolState::new();
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local entries = tools.journal(nil, 10)
            assert(#entries == 0, "no shared state should return empty list")
        "#,
        )
        .exec()
        .expect("journal without shared state should return empty");
    }

    #[test]
    fn test_journal_with_shared_state() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let instance = TestInstance::new().expect("should create instance");

        rt.block_on(async {
            instance.create_room("testroom", None).await;
            instance
                .add_journal("testroom", JournalKind::Note, "Test note")
                .await;
            instance
                .add_journal("testroom", JournalKind::Decision, "Test decision")
                .await;
        });

        let lua = Lua::new();
        let state = instance.lua_tool_state("testroom");
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local entries = tools.journal(nil, 10)
            assert(#entries == 2, "should have 2 journal entries")

            -- Filter by kind
            local notes = tools.journal("note", 10)
            assert(#notes == 1, "should have 1 note")
        "#,
        )
        .exec()
        .expect("journal with shared state should work");
    }

    #[test]
    fn test_inspirations_without_shared_state() {
        let lua = Lua::new();
        let state = LuaToolState::new();
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local inspirations = tools.inspirations()
            assert(#inspirations == 0, "no shared state should return empty list")
        "#,
        )
        .exec()
        .expect("inspirations without shared state should return empty");
    }

    #[test]
    fn test_inspirations_with_shared_state() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let instance = TestInstance::new().expect("should create instance");

        rt.block_on(async {
            instance.create_room("testroom", None).await;
            instance.add_inspiration("testroom", "Be creative!").await;
        });

        let lua = Lua::new();
        let state = instance.lua_tool_state("testroom");
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local inspirations = tools.inspirations()
            assert(#inspirations == 1, "should have 1 inspiration")
            assert(inspirations[1].content == "Be creative!", "content")
        "#,
        )
        .exec()
        .expect("inspirations with shared state should work");
    }

    #[test]
    fn test_rooms_without_shared_state() {
        let lua = Lua::new();
        let state = LuaToolState::new();
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local rooms = tools.rooms()
            assert(#rooms == 0, "no shared state should return empty list")
        "#,
        )
        .exec()
        .expect("rooms without shared state should return empty");
    }

    #[test]
    fn test_rooms_with_shared_state() {
        // Create instance synchronously using tokio runtime block
        let rt = tokio::runtime::Runtime::new().unwrap();
        let instance = TestInstance::new().expect("should create instance");

        // Create rooms synchronously using blocking runtime
        rt.block_on(async {
            instance.create_room("room1", None).await;
            instance.create_room("room2", Some("A vibey room")).await;
        });

        // Now run Lua test outside the async context
        let lua = Lua::new();
        let state = instance.lua_tool_state("room1");
        register_tools(&lua, state).expect("should register tools");

        lua.load(
            r#"
            local rooms = tools.rooms()
            assert(#rooms == 2, "should have 2 rooms")
        "#,
        )
        .exec()
        .expect("rooms with shared state should work");
    }
}
