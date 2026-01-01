//! Lua integration for sshwarma
//!
//! This module provides Lua scripting support for customizable HUD rendering.
//! Supports hot-reloading of user scripts from ~/.config/sshwarma/hud.lua.

pub mod cache;
pub mod context;
pub mod data;
pub mod mcp_bridge;
pub mod render;
pub mod tool_middleware;
pub mod tools;
pub mod wrap;

pub use cache::ToolCache;
pub use context::{build_hud_context, PendingNotification};
pub use mcp_bridge::{mcp_request_handler, McpBridge};
pub use render::{parse_lua_output, HUD_ROWS};
pub use tool_middleware::{ToolContext, ToolMiddleware};
pub use tools::{register_mcp_tools, LuaToolState};
pub use wrap::{compose_context, WrapResult, WrapState};

// Re-export startup script path for main.rs
pub use self::startup_script_path as get_startup_script_path;

use crate::display::hud::HudState;
use crate::lua::tools::register_tools;
use crate::paths;
use anyhow::{Context, Result};
use mlua::{Lua, Table, Value};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use tracing::{debug, info, warn};

/// Embedded default HUD script
const DEFAULT_HUD_SCRIPT: &str = include_str!("../embedded/hud.lua");

/// Embedded wrap script for context composition
const DEFAULT_WRAP_SCRIPT: &str = include_str!("../embedded/wrap.lua");

/// Path to user's custom HUD script
pub fn user_hud_script_path() -> PathBuf {
    paths::config_dir().join("hud.lua")
}

/// Path to a specific user's HUD script (e.g., atobey.lua, claude.lua)
pub fn user_named_script_path(username: &str) -> PathBuf {
    paths::config_dir().join(format!("{}.lua", username))
}

/// Path to server startup script
pub fn startup_script_path() -> PathBuf {
    paths::config_dir().join("startup.lua")
}

/// Lua runtime for HUD rendering
///
/// Manages the Lua state, script loading, and hot-reloading.
/// Provides `render_hud()` to generate HUD output from Lua.
pub struct LuaRuntime {
    /// The Lua interpreter state
    lua: Lua,
    /// Tool state shared with Lua callbacks
    tool_state: LuaToolState,
    /// Path to currently loaded user script (None = embedded default)
    loaded_script_path: Option<PathBuf>,
    /// Last modification time of loaded script
    loaded_script_mtime: Option<SystemTime>,
}

impl LuaRuntime {
    /// Create a new Lua runtime with the default embedded script
    pub fn new() -> Result<Self> {
        let lua = Lua::new();
        let tool_state = LuaToolState::new();

        // Register tool functions (creates the tools table)
        register_tools(&lua, tool_state.clone())
            .map_err(|e| anyhow::anyhow!("failed to register Lua tools: {}", e))?;

        // Load the wrap script first (provides wrap() and default_wrap())
        lua.load(DEFAULT_WRAP_SCRIPT)
            .set_name("embedded:wrap.lua")
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to load embedded wrap script: {}", e))?;

        // Load the default HUD script
        lua.load(DEFAULT_HUD_SCRIPT)
            .set_name("embedded:hud.lua")
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to load embedded HUD script: {}", e))?;

        info!("Lua runtime initialized with embedded scripts");

        Ok(Self {
            lua,
            tool_state,
            loaded_script_path: None,
            loaded_script_mtime: None,
        })
    }

    /// Create a new Lua runtime, trying to load user script first
    ///
    /// Falls back to embedded script if user script doesn't exist or fails.
    pub fn new_with_user_script() -> Result<Self> {
        Self::new_for_user(None)
    }

    /// Create a new Lua runtime for a specific user
    ///
    /// Script lookup order:
    /// 1. `{username}.lua` (e.g., atobey.lua, claude.lua)
    /// 2. `hud.lua` (shared fallback)
    /// 3. Embedded default
    pub fn new_for_user(username: Option<&str>) -> Result<Self> {
        let mut runtime = Self::new()?;

        // Try user-specific script first (e.g., atobey.lua)
        if let Some(name) = username {
            let named_path = user_named_script_path(name);
            if named_path.exists() {
                match runtime.load_script(&named_path) {
                    Ok(()) => {
                        info!("Loaded HUD script for '{}' from {:?}", name, named_path);
                        return Ok(runtime);
                    }
                    Err(e) => {
                        warn!(
                            "Failed to load HUD script {:?}: {}. Trying fallback.",
                            named_path, e
                        );
                    }
                }
            }
        }

        // Try shared user script (hud.lua)
        let user_path = user_hud_script_path();
        if user_path.exists() {
            match runtime.load_script(&user_path) {
                Ok(()) => {
                    info!("Loaded user HUD script from {:?}", user_path);
                }
                Err(e) => {
                    warn!(
                        "Failed to load user HUD script {:?}: {}. Using embedded default.",
                        user_path, e
                    );
                }
            }
        } else {
            debug!(
                "No user HUD script at {:?}, using embedded default",
                user_path
            );
        }

        Ok(runtime)
    }

    /// Run the startup script if it exists
    ///
    /// Looks for `~/.config/sshwarma/startup.lua` and executes it.
    /// The script can call `tools.mcp_connect()`, `tools.mcp_disconnect()`, etc.
    ///
    /// Returns Ok(true) if script was run, Ok(false) if no script exists.
    pub fn run_startup_script(&self) -> Result<bool> {
        let script_path = startup_script_path();

        if !script_path.exists() {
            debug!("No startup script at {:?}", script_path);
            return Ok(false);
        }

        let script = fs::read_to_string(&script_path)
            .with_context(|| format!("failed to read startup script {:?}", script_path))?;

        info!("Running startup script from {:?}", script_path);

        self.lua
            .load(&script)
            .set_name(script_path.to_string_lossy())
            .exec()
            .map_err(|e| anyhow::anyhow!("startup script failed: {}", e))?;

        // Check if there's a startup() function and call it
        let globals = self.lua.globals();
        let startup_fn: Value = globals
            .get("startup")
            .map_err(|e| anyhow::anyhow!("failed to get startup: {}", e))?;

        if startup_fn != Value::Nil {
            let func: mlua::Function = startup_fn
                .as_function()
                .ok_or_else(|| anyhow::anyhow!("startup is not a function"))?
                .clone();

            func.call::<()>(())
                .map_err(|e| anyhow::anyhow!("startup() call failed: {}", e))?;

            info!("Startup script completed successfully");
        } else {
            info!("Startup script executed (no startup() function defined)");
        }

        Ok(true)
    }

    /// Load a Lua script from file
    ///
    /// Replaces the current script. The script must define a `render_hud` function.
    pub fn load_script(&mut self, path: &PathBuf) -> Result<()> {
        let script = fs::read_to_string(path)
            .with_context(|| format!("failed to read script {:?}", path))?;

        let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();

        // Load and execute the script
        self.lua
            .load(&script)
            .set_name(path.to_string_lossy())
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to execute script {:?}: {}", path, e))?;

        // Verify render_hud function exists
        let globals = self.lua.globals();
        let render_hud: Value = globals
            .get("render_hud")
            .map_err(|e| anyhow::anyhow!("failed to get render_hud: {}", e))?;
        if render_hud == Value::Nil {
            anyhow::bail!("script must define a render_hud function");
        }

        self.loaded_script_path = Some(path.clone());
        self.loaded_script_mtime = mtime;

        debug!("Loaded HUD script from {:?}", path);
        Ok(())
    }

    /// Check if the loaded script has been modified and reload if needed
    ///
    /// Returns true if script was reloaded.
    pub fn check_reload(&mut self) -> bool {
        let Some(ref path) = self.loaded_script_path else {
            return false;
        };

        // Check if file still exists
        let Ok(metadata) = fs::metadata(path) else {
            // File was deleted, reload embedded default
            warn!(
                "User script {:?} was deleted, reverting to embedded default",
                path
            );
            if let Err(e) = self.reload_embedded() {
                warn!("Failed to reload embedded script: {}", e);
            }
            return true;
        };

        // Check mtime
        let current_mtime = metadata.modified().ok();
        if current_mtime != self.loaded_script_mtime {
            info!("User script {:?} was modified, reloading", path);
            let path_clone = path.clone();
            match self.load_script(&path_clone) {
                Ok(()) => {
                    info!("Successfully reloaded user script");
                    return true;
                }
                Err(e) => {
                    warn!(
                        "Failed to reload user script: {}. Keeping previous version.",
                        e
                    );
                }
            }
        }

        false
    }

    /// Reload the embedded default script
    fn reload_embedded(&mut self) -> Result<()> {
        self.lua
            .load(DEFAULT_HUD_SCRIPT)
            .set_name("embedded:hud.lua")
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to reload embedded HUD script: {}", e))?;

        self.loaded_script_path = None;
        self.loaded_script_mtime = None;

        Ok(())
    }

    /// Update the HUD state before rendering
    ///
    /// Call this before `render_hud()` to provide current state.
    pub fn update_state(&self, state: HudState) {
        self.tool_state.update_hud_state(state);
    }

    /// Push a notification for the HUD to display
    pub fn push_notification(&self, message: String, ttl_ms: i64) {
        self.tool_state.push_notification(message, ttl_ms);
    }

    /// Get a reference to the tool state for cache access
    pub fn tool_state(&self) -> &LuaToolState {
        &self.tool_state
    }

    /// Get a reference to the cache for background updates
    pub fn cache(&self) -> &ToolCache {
        self.tool_state.cache()
    }

    /// Get a reference to the Lua state
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Render the HUD by calling Lua's render_hud function
    ///
    /// Returns the raw Lua table result. Use `parse_lua_output()` to convert
    /// this to an ANSI-escaped string for terminal display.
    ///
    /// # Arguments
    /// - `now_ms`: Current time in milliseconds (for animations)
    /// - `width`: Terminal width
    /// - `height`: Terminal height
    ///
    /// # Returns
    /// Lua table with 8 rows of segments
    pub fn render_hud(&self, now_ms: i64, width: u16, height: u16) -> Result<Table> {
        let globals = self.lua.globals();
        let render_fn: mlua::Function = globals
            .get("render_hud")
            .map_err(|e| anyhow::anyhow!("render_hud function not found: {}", e))?;

        let result: Table = render_fn
            .call((now_ms, width as i64, height as i64))
            .map_err(|e| anyhow::anyhow!("render_hud call failed: {}", e))?;

        Ok(result)
    }

    /// Render the HUD and convert to ANSI string
    ///
    /// Convenience method that calls `render_hud()` and then `parse_lua_output()`.
    ///
    /// # Arguments
    /// - `now_ms`: Current time in milliseconds (for animations)
    /// - `width`: Terminal width
    /// - `height`: Terminal height
    ///
    /// # Returns
    /// 8 lines joined by CRLF with ANSI color codes
    pub fn render_hud_string(&self, now_ms: i64, width: u16, height: u16) -> Result<String> {
        let table = self.render_hud(now_ms, width, height)?;
        parse_lua_output(table)
    }

    /// Compose context for LLM interactions
    ///
    /// Calls Lua's `default_wrap()` function with the target token budget,
    /// then extracts system_prompt and context strings.
    ///
    /// # Arguments
    /// - `wrap_state`: State needed for context composition (room, user, model)
    /// - `target_tokens`: Token budget for context (e.g., model's context_window)
    ///
    /// # Returns
    /// WrapResult with system_prompt (stable, for preamble) and context (dynamic)
    pub fn compose_context(
        &self,
        wrap_state: WrapState,
        target_tokens: usize,
    ) -> Result<WrapResult> {
        use crate::lua::tools::SessionContext;

        // Set session context for unified tools to access
        self.tool_state.set_session_context(Some(SessionContext {
            username: wrap_state.username.clone(),
            model: Some(wrap_state.model.clone()),
            room_name: wrap_state.room_name.clone(),
        }));

        // Set shared state for extended data tools
        self.tool_state
            .set_shared_state(Some(wrap_state.shared_state.clone()));

        // Also set HudState with room_name so tools.history() etc. can find the room
        // (tools use HudState.room_name to determine which room to read from)
        {
            let mut hud = self.tool_state.hud_state();
            hud.room_name = wrap_state.room_name.clone();
            self.tool_state.update_hud_state(hud);
        }

        // Call compose_context from wrap.rs
        let result = wrap::compose_context(&self.lua, target_tokens);

        // Cleanup session context (shared_state can persist for HUD)
        self.tool_state.clear_session_context();

        result
    }

    /// Render room look with ANSI formatting (for TTY /look command)
    ///
    /// Calls Lua's `look_ansi()` function which returns segment tables,
    /// then converts to ANSI string for terminal display.
    pub fn render_look_ansi(&self, wrap_state: WrapState) -> Result<String> {
        use crate::lua::tools::SessionContext;

        // Set session context for tools to access
        self.tool_state.set_session_context(Some(SessionContext {
            username: wrap_state.username.clone(),
            model: None, // Not needed for look
            room_name: wrap_state.room_name.clone(),
        }));

        // Set shared state for extended data tools
        self.tool_state
            .set_shared_state(Some(wrap_state.shared_state.clone()));

        // Populate HudState with room data from SharedState
        {
            let mut hud = self.tool_state.hud_state();
            hud.room_name = wrap_state.room_name.clone();

            // Fetch room data if we have a room name
            if let Some(ref room_name) = wrap_state.room_name {
                let world =
                    tokio::task::block_in_place(|| wrap_state.shared_state.world.blocking_read());
                if let Some(room) = world.get_room(room_name) {
                    // Vibe and exits are stored in DB, not always in memory
                    hud.vibe = wrap_state
                        .shared_state
                        .db
                        .get_vibe(room_name)
                        .ok()
                        .flatten();
                    hud.exits = wrap_state
                        .shared_state
                        .db
                        .get_exits(room_name)
                        .unwrap_or_default();
                    hud.description = room.description.clone();

                    // Build participants list
                    let users: Vec<String> = room.users.to_vec();
                    let models: Vec<String> = wrap_state
                        .shared_state
                        .models
                        .available()
                        .iter()
                        .map(|m| m.short_name.clone())
                        .collect();
                    hud.set_participants(users, models);
                }
            }

            self.tool_state.update_hud_state(hud);
        }

        // Call Lua's look_ansi() function
        let globals = self.lua.globals();
        let look_ansi_fn: mlua::Function = globals
            .get("look_ansi")
            .map_err(|e| anyhow::anyhow!("look_ansi function not found: {}", e))?;

        let rows: Table = look_ansi_fn
            .call(())
            .map_err(|e| anyhow::anyhow!("look_ansi call failed: {}", e))?;

        // Parse segments to ANSI string
        let output = render::parse_lua_rows(rows)?;

        // Cleanup
        self.tool_state.clear_session_context();

        Ok(output)
    }

    /// Render room look as markdown (for model sshwarma_look tool)
    ///
    /// Calls Lua's `look_markdown()` function which returns a markdown string.
    pub fn render_look_markdown(&self, wrap_state: WrapState) -> Result<String> {
        use crate::lua::tools::SessionContext;

        // Set session context for tools to access
        self.tool_state.set_session_context(Some(SessionContext {
            username: wrap_state.username.clone(),
            model: None,
            room_name: wrap_state.room_name.clone(),
        }));

        // Set shared state for extended data tools
        self.tool_state
            .set_shared_state(Some(wrap_state.shared_state.clone()));

        // Populate HudState with room data from SharedState
        {
            let mut hud = self.tool_state.hud_state();
            hud.room_name = wrap_state.room_name.clone();

            // Fetch room data if we have a room name
            if let Some(ref room_name) = wrap_state.room_name {
                let world =
                    tokio::task::block_in_place(|| wrap_state.shared_state.world.blocking_read());
                if let Some(room) = world.get_room(room_name) {
                    // Vibe and exits are stored in DB, not always in memory
                    hud.vibe = wrap_state
                        .shared_state
                        .db
                        .get_vibe(room_name)
                        .ok()
                        .flatten();
                    hud.exits = wrap_state
                        .shared_state
                        .db
                        .get_exits(room_name)
                        .unwrap_or_default();
                    hud.description = room.description.clone();

                    // Build participants list
                    let users: Vec<String> = room.users.to_vec();
                    let models: Vec<String> = wrap_state
                        .shared_state
                        .models
                        .available()
                        .iter()
                        .map(|m| m.short_name.clone())
                        .collect();
                    hud.set_participants(users, models);
                }
            }

            self.tool_state.update_hud_state(hud);
        }

        // Call Lua's look_markdown() function
        let globals = self.lua.globals();
        let look_markdown_fn: mlua::Function = globals
            .get("look_markdown")
            .map_err(|e| anyhow::anyhow!("look_markdown function not found: {}", e))?;

        let result: String = look_markdown_fn
            .call(())
            .map_err(|e| anyhow::anyhow!("look_markdown call failed: {}", e))?;

        // Cleanup
        self.tool_state.clear_session_context();

        Ok(result)
    }

    /// Check if a user script is loaded (vs embedded default)
    pub fn has_user_script(&self) -> bool {
        self.loaded_script_path.is_some()
    }

    /// Get the path to the loaded user script, if any
    pub fn user_script_path(&self) -> Option<&PathBuf> {
        self.loaded_script_path.as_ref()
    }

    /// Call the background(tick) function if it exists
    ///
    /// This is called on a timer (e.g., 500ms at 120 BPM) to allow Lua scripts
    /// to poll MCP tools and update state. The tick counter can be used for
    /// subdivision timing (tick % 4 == 0 for every 4 ticks, etc.).
    ///
    /// If the script doesn't define a `background` function, this is a no-op.
    pub fn call_background(&self, tick: u64) -> Result<()> {
        let globals = self.lua.globals();

        // Check if background function exists
        let background_fn: Value = globals
            .get("background")
            .map_err(|e| anyhow::anyhow!("failed to get background: {}", e))?;

        if background_fn == Value::Nil {
            // No background function defined, this is fine
            return Ok(());
        }

        let func: mlua::Function = background_fn
            .as_function()
            .ok_or_else(|| anyhow::anyhow!("background is not a function"))?
            .clone();

        func.call::<()>(tick)
            .map_err(|e| anyhow::anyhow!("background() call failed: {}", e))?;

        Ok(())
    }

    /// Check if the script defines a background() function
    pub fn has_background(&self) -> bool {
        self.lua
            .globals()
            .get::<Value>("background")
            .map(|v| v != Value::Nil)
            .unwrap_or(false)
    }
}

impl Default for LuaRuntime {
    fn default() -> Self {
        Self::new().expect("failed to create default LuaRuntime")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::db::Database;
    use crate::display::EntryContent;
    use crate::display::EntrySource;
    use crate::llm::LlmClient;
    use crate::mcp::McpManager;
    use crate::model::{ModelBackend, ModelHandle, ModelRegistry};
    use crate::state::SharedState;
    use crate::world::World;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// Reusable test fixture with real components (mirrors tools.rs TestInstance)
    struct TestInstance {
        shared_state: Arc<SharedState>,
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
                db,
                config: Config::default(),
                llm: Arc::new(LlmClient::new()?),
                models: models.clone(),
                mcp: Arc::new(McpManager::new()),
            });

            Ok(Self {
                shared_state,
                world,
                models,
            })
        }

        async fn create_room(&self, name: &str, vibe: Option<&str>) {
            let mut world = self.world.write().await;
            world.create_room(name.to_string());
            if let Some(v) = vibe {
                if let Some(room) = world.get_room_mut(name) {
                    room.context.vibe = Some(v.to_string());
                }
            }
        }

        async fn add_message(&self, room: &str, sender: &str, content: &str) {
            let mut world = self.world.write().await;
            if let Some(r) = world.get_room_mut(room) {
                r.ledger.push(
                    EntrySource::User(sender.to_string()),
                    EntryContent::Chat(content.to_string()),
                );
            }
        }
    }

    #[test]
    fn test_lua_runtime_new() {
        let runtime = LuaRuntime::new().expect("should create runtime");
        assert!(!runtime.has_user_script());
    }

    #[test]
    fn test_render_hud_default() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Update with empty state
        runtime.update_state(HudState::new());

        // Render
        let now_ms = chrono::Utc::now().timestamp_millis();
        let table = runtime.render_hud(now_ms, 80, 8).expect("should render");

        // Should have 8 rows
        let mut row_count = 0;
        for i in 1..=8 {
            if table.get::<Value>(i).is_ok() {
                row_count += 1;
            }
        }
        assert_eq!(row_count, 8, "HUD should have 8 rows");
    }

    #[test]
    fn test_render_hud_string() {
        let runtime = LuaRuntime::new().expect("should create runtime");
        runtime.update_state(HudState::new());

        let now_ms = chrono::Utc::now().timestamp_millis();
        let output = runtime
            .render_hud_string(now_ms, 80, 8)
            .expect("should render string");

        let lines: Vec<&str> = output.split("\r\n").collect();
        assert_eq!(lines.len(), 8, "Should have 8 lines");
    }

    #[test]
    fn test_notification_queue() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Push a notification
        runtime.push_notification("Test notification".to_string(), 5000);

        // The notification will be processed on next render
    }

    #[test]
    fn test_user_config_path() {
        let path = user_hud_script_path();
        assert!(path.to_string_lossy().contains("sshwarma"));
        assert!(path.to_string_lossy().ends_with("hud.lua"));
    }

    #[test]
    fn test_cache_access() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Should be able to get cache
        let cache = runtime.cache();
        cache.set_blocking("test_key".to_string(), serde_json::json!({"foo": "bar"}));

        let value = cache.get_data_blocking("test_key");
        assert!(value.is_some());
        assert_eq!(value.unwrap(), serde_json::json!({"foo": "bar"}));
    }

    // =========================================================================
    // compose_context integration tests
    // =========================================================================

    #[test]
    fn test_compose_context_basic() {
        use crate::lua::wrap::WrapState;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let instance = TestInstance::new().expect("should create instance");

        rt.block_on(async {
            instance
                .create_room("testroom", Some("Creative jam session"))
                .await;
            instance
                .add_message("testroom", "alice", "Let's make music!")
                .await;
        });

        let model = instance.models.get("test").unwrap().clone();

        let runtime = LuaRuntime::new().expect("should create runtime");
        // Set up tool state for compose_context
        runtime
            .tool_state
            .set_shared_state(Some(instance.shared_state.clone()));

        let wrap_state = WrapState {
            room_name: Some("testroom".to_string()),
            username: "alice".to_string(),
            model,
            shared_state: instance.shared_state.clone(),
        };

        let result = runtime
            .compose_context(wrap_state, 8000)
            .expect("should compose context");

        // System prompt should contain model identity and sshwarma info
        assert!(
            result.system_prompt.contains("@test"),
            "system prompt should contain model handle"
        );
        assert!(
            result.system_prompt.contains("sshwarma"),
            "system prompt should contain sshwarma"
        );

        // Context should contain room info or user info
        // (room context is dynamic, so it goes to context not system_prompt)
        assert!(
            result.context.contains("testroom") || result.context.contains("alice"),
            "context should contain room or user info"
        );
    }

    #[test]
    fn test_compose_context_with_history() {
        use crate::lua::wrap::WrapState;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let instance = TestInstance::new().expect("should create instance");

        rt.block_on(async {
            instance.create_room("testroom", None).await;
            instance
                .add_message("testroom", "bob", "Hello everyone!")
                .await;
            instance
                .add_message("testroom", "alice", "Hi bob, what's up?")
                .await;
        });

        let model = instance.models.get("test").unwrap().clone();

        let runtime = LuaRuntime::new().expect("should create runtime");
        runtime
            .tool_state
            .set_shared_state(Some(instance.shared_state.clone()));

        let wrap_state = WrapState {
            room_name: Some("testroom".to_string()),
            username: "alice".to_string(),
            model,
            shared_state: instance.shared_state.clone(),
        };

        let result = runtime
            .compose_context(wrap_state, 8000)
            .expect("should compose context");

        // Context should contain history with messages
        assert!(
            result.context.contains("Hello everyone") || result.context.contains("bob"),
            "context should contain chat history"
        );
    }

    #[test]
    fn test_compose_context_budget_overflow() {
        use crate::lua::wrap::WrapState;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let instance = TestInstance::new().expect("should create instance");

        rt.block_on(async {
            instance.create_room("testroom", None).await;
            // Add many messages to exceed a tiny budget
            for i in 0..100 {
                instance
                    .add_message(
                        "testroom",
                        "user",
                        &format!("This is message number {} with some extra content to make it longer and use more tokens", i),
                    )
                    .await;
            }
        });

        let model = instance.models.get("test").unwrap().clone();

        let runtime = LuaRuntime::new().expect("should create runtime");
        runtime
            .tool_state
            .set_shared_state(Some(instance.shared_state.clone()));

        let wrap_state = WrapState {
            room_name: Some("testroom".to_string()),
            username: "alice".to_string(),
            model,
            shared_state: instance.shared_state.clone(),
        };

        // Very small budget should trigger overflow error
        let result = runtime.compose_context(wrap_state, 100);
        assert!(result.is_err(), "should error on budget overflow");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Compaction required") || err_msg.contains("exceeds budget"),
            "error should mention compaction or budget: {}",
            err_msg
        );
    }

    // =========================================================================
    // wrap.lua Lua-level tests
    // =========================================================================

    #[test]
    fn test_wrap_builder_chaining() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        runtime
            .lua()
            .load(
                r#"
            local w = wrap(10000)
                :system()
                :model_identity()
                :room()

            -- Should have 3 sources
            assert(#w.sources == 3, "should have 3 sources, got " .. #w.sources)

            -- Check priorities are set correctly
            assert(w.sources[1].priority == 0, "system priority should be 0")
            assert(w.sources[2].priority == 10, "model priority should be 10")
            assert(w.sources[3].priority == 20, "room priority should be 20")
        "#,
            )
            .exec()
            .expect("wrap builder chaining should work");
    }

    #[test]
    fn test_wrap_builder_custom_source() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        runtime
            .lua()
            .load(
                r#"
            local w = wrap(10000)
                :custom("my_context", "This is custom content", 50, false)

            assert(#w.sources == 1, "should have 1 source")
            assert(w.sources[1].name == "my_context", "name should be my_context")
            assert(w.sources[1].priority == 50, "priority should be 50")
            assert(w.sources[1].is_system == false, "should not be system")
        "#,
            )
            .exec()
            .expect("custom source should work");
    }

    #[test]
    fn test_wrap_system_vs_context_separation() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        runtime
            .lua()
            .load(
                r#"
            local w = wrap(10000)
                :system()           -- is_system = true
                :model_identity()   -- is_system = true
                :room()             -- is_system = false
                :history(10)        -- is_system = false

            -- Count system vs context sources
            local system_count = 0
            local context_count = 0
            for _, source in ipairs(w.sources) do
                if source.is_system then
                    system_count = system_count + 1
                else
                    context_count = context_count + 1
                end
            end

            assert(system_count == 2, "should have 2 system sources, got " .. system_count)
            assert(context_count == 2, "should have 2 context sources, got " .. context_count)
        "#,
            )
            .exec()
            .expect("system vs context separation should work");
    }

    #[test]
    fn test_wrap_default_wrap_function() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        runtime
            .lua()
            .load(
                r#"
            -- default_wrap should create a fully configured builder
            local w = default_wrap(10000)

            -- Should have multiple sources (system, model, user, room, participants, etc.)
            assert(#w.sources >= 5, "default_wrap should have at least 5 sources, got " .. #w.sources)

            -- Should have target_tokens set
            assert(w.target_tokens == 10000, "target_tokens should be 10000")
        "#,
            )
            .exec()
            .expect("default_wrap should work");
    }

    #[test]
    fn test_wrap_estimate_tokens() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        runtime
            .lua()
            .load(
                r#"
            -- estimate_tokens uses ~4 chars per token heuristic
            local result = tools.estimate_tokens("hello world test")  -- 16 chars
            assert(result == 4, "16 chars should be 4 tokens, got " .. tostring(result))

            local result2 = tools.estimate_tokens("")
            assert(result2 == 0, "empty string should be 0 tokens")
        "#,
            )
            .exec()
            .expect("estimate_tokens should work");
    }
}
