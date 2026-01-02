//! Lua integration for sshwarma
//!
//! This module provides Lua scripting support for customizable HUD rendering.
//! Supports hot-reloading of user scripts from ~/.config/sshwarma/hud.lua.

pub mod cache;
pub mod context;
pub mod data;
pub mod mcp_bridge;
pub mod registry;
pub mod render;
pub mod tool_middleware;
pub mod tools;
pub mod wrap;

pub use cache::ToolCache;
pub use context::{NotificationLevel, PendingNotification};
pub use mcp_bridge::{mcp_request_handler, McpBridge};
pub use registry::ToolRegistry;
pub use render::parse_lua_rows;
pub use tool_middleware::{ToolContext, ToolMiddleware};
pub use tools::{register_mcp_tools, InputState, LuaToolState, SessionContext};
pub use wrap::{compose_context, WrapResult, WrapState};

// Re-export startup script path for main.rs
pub use self::startup_script_path as get_startup_script_path;

use crate::lua::tools::register_tools;
use crate::paths;
use anyhow::{Context, Result};
use mlua::{Lua, Table, Value};
use opentelemetry::KeyValue;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use tracing::{debug, info, warn};

/// Counter for Lua script errors
fn lua_error_counter() -> opentelemetry::metrics::Counter<u64> {
    static COUNTER: std::sync::OnceLock<opentelemetry::metrics::Counter<u64>> =
        std::sync::OnceLock::new();
    COUNTER
        .get_or_init(|| {
            opentelemetry::global::meter("sshwarma")
                .u64_counter("lua_errors")
                .with_description("Count of Lua script errors by type")
                .build()
        })
        .clone()
}

/// Record a Lua error with context
fn record_lua_error(error_type: &str, error_msg: &str) {
    lua_error_counter().add(
        1,
        &[
            KeyValue::new("error_type", error_type.to_string()),
            KeyValue::new("error_class", classify_lua_error(error_msg).to_string()),
        ],
    );
    tracing::error!(error_type = error_type, "Lua error: {}", error_msg);
}

/// Classify Lua errors for grouping in metrics
fn classify_lua_error(msg: &str) -> &'static str {
    if msg.contains("attempt to index") || msg.contains("attempt to call") {
        "nil_access"
    } else if msg.contains("stack overflow") {
        "stack_overflow"
    } else if msg.contains("syntax error") || msg.contains("unexpected") {
        "syntax"
    } else if msg.contains("timeout") {
        "timeout"
    } else {
        "runtime"
    }
}

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
/// Lua scripts implement `on_tick(tick, ctx)` to render the HUD.
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

        // Register sshwarma.call() unified interface
        tools::register_sshwarma_call(&lua, tool_state.clone())
            .map_err(|e| anyhow::anyhow!("failed to register sshwarma.call: {}", e))?;

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
    /// Replaces the current script. The script should define an `on_tick` function.
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

        // Verify on_tick function exists (optional - script can define just background)
        let globals = self.lua.globals();
        let on_tick: Value = globals
            .get("on_tick")
            .map_err(|e| anyhow::anyhow!("failed to get on_tick: {}", e))?;
        if on_tick == Value::Nil {
            warn!("Script {:?} does not define on_tick function", path);
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

    /// Push a notification for the HUD to display
    pub fn push_notification(&self, message: String, ttl_ms: i64) {
        self.tool_state.push_notification(message, ttl_ms);
    }

    /// Push a notification with a specific level
    pub fn push_notification_with_level(
        &self,
        message: String,
        ttl_ms: i64,
        level: NotificationLevel,
    ) {
        self.tool_state
            .push_notification_with_level(message, ttl_ms, level);
    }

    /// Push an error notification (convenience method)
    pub fn push_error(&self, message: String) {
        self.push_notification_with_level(message, 10000, NotificationLevel::Error);
    }

    /// Push a warning notification (convenience method)
    pub fn push_warning(&self, message: String) {
        self.push_notification_with_level(message, 7000, NotificationLevel::Warning);
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

        // Save current session context (don't clobber HUD's context)
        let saved_context = self.tool_state.session_context();

        // Set session context for unified tools to access
        self.tool_state.set_session_context(Some(SessionContext {
            username: wrap_state.username.clone(),
            model: Some(wrap_state.model.clone()),
            room_name: wrap_state.room_name.clone(),
        }));

        // Set shared state for extended data tools
        self.tool_state
            .set_shared_state(Some(wrap_state.shared_state.clone()));

        // Call compose_context from wrap.rs
        let result = wrap::compose_context(&self.lua, target_tokens);

        // Restore previous session context
        self.tool_state.set_session_context(saved_context);

        result
    }

    /// Render room look with ANSI formatting (for TTY /look command)
    ///
    /// Calls Lua's `look_ansi()` function which returns segment tables,
    /// then converts to ANSI string for terminal display.
    pub fn render_look_ansi(&self, wrap_state: WrapState) -> Result<String> {
        use crate::lua::tools::SessionContext;

        // Save current session context (don't clobber HUD's context)
        let saved_context = self.tool_state.session_context();

        // Set session context for tools to access
        self.tool_state.set_session_context(Some(SessionContext {
            username: wrap_state.username.clone(),
            model: None, // Not needed for look
            room_name: wrap_state.room_name.clone(),
        }));

        // Set shared state for extended data tools
        // Lua queries room data via sshwarma.call("status") or sshwarma.call("room")
        self.tool_state
            .set_shared_state(Some(wrap_state.shared_state.clone()));

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

        // Restore previous session context
        self.tool_state.set_session_context(saved_context);

        Ok(output)
    }

    /// Render room look as markdown (for model sshwarma_look tool)
    ///
    /// Calls Lua's `look_markdown()` function which returns a markdown string.
    pub fn render_look_markdown(&self, wrap_state: WrapState) -> Result<String> {
        use crate::lua::tools::SessionContext;

        // Save current session context (don't clobber HUD's context)
        let saved_context = self.tool_state.session_context();

        // Set session context for tools to access
        self.tool_state.set_session_context(Some(SessionContext {
            username: wrap_state.username.clone(),
            model: None,
            room_name: wrap_state.room_name.clone(),
        }));

        // Set shared state for extended data tools
        // Lua queries room data via sshwarma.call("status") or sshwarma.call("room")
        self.tool_state
            .set_shared_state(Some(wrap_state.shared_state.clone()));

        // Call Lua's look_markdown() function
        let globals = self.lua.globals();
        let look_markdown_fn: mlua::Function = globals
            .get("look_markdown")
            .map_err(|e| anyhow::anyhow!("look_markdown function not found: {}", e))?;

        let result: String = look_markdown_fn
            .call(())
            .map_err(|e| anyhow::anyhow!("look_markdown call failed: {}", e))?;

        // Restore previous session context
        self.tool_state.set_session_context(saved_context);

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

    /// Call the on_tick(tick, ctx) function if it exists
    ///
    /// This is the main render loop entry point. Called every 100ms.
    /// Lua receives:
    /// - tick: monotonic tick counter
    /// - ctx: LuaDrawContext for the full screen
    ///
    /// Lua can draw to the context, and the caller will diff and emit output.
    /// If the script doesn't define an `on_tick` function, this is a no-op.
    pub fn call_on_tick(
        &self,
        tick: u64,
        render_buffer: std::sync::Arc<std::sync::Mutex<crate::ui::RenderBuffer>>,
        width: u16,
        height: u16,
    ) -> Result<()> {
        let globals = self.lua.globals();

        // Check if on_tick function exists
        let on_tick_fn: Value = globals
            .get("on_tick")
            .map_err(|e| anyhow::anyhow!("failed to get on_tick: {}", e))?;

        if on_tick_fn == Value::Nil {
            // No on_tick function defined, this is fine
            return Ok(());
        }

        let func: mlua::Function = on_tick_fn
            .as_function()
            .ok_or_else(|| anyhow::anyhow!("on_tick is not a function"))?
            .clone();

        // Create draw context for full screen
        let ctx = crate::ui::LuaDrawContext::new(render_buffer, 0, 0, width, height);

        func.call::<()>((tick, ctx)).map_err(|e| {
            let msg = format!("on_tick() call failed: {}", e);
            record_lua_error("on_tick", &msg);
            anyhow::anyhow!("{}", msg)
        })?;

        Ok(())
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

        func.call::<()>(tick).map_err(|e| {
            let msg = format!("background() call failed: {}", e);
            record_lua_error("background", &msg);
            anyhow::anyhow!("{}", msg)
        })?;

        Ok(())
    }

    /// Execute a rule script's handler function
    ///
    /// Rule scripts are stored in the database and define a `handle(tick, state)`
    /// function that receives the tick number and room state.
    ///
    /// Returns the result of the handler as a Lua value for processing
    /// (e.g., for notify slots that return notification tables).
    pub fn execute_rule_script(
        &self,
        script_code: &str,
        script_name: &str,
        tick: u64,
    ) -> Result<Option<Table>> {
        // Load and execute the script to define the handle function
        self.lua
            .load(script_code)
            .set_name(format!("rule:{}", script_name))
            .exec()
            .map_err(|e| {
                let msg = format!("failed to load rule script '{}': {}", script_name, e);
                record_lua_error("rule_load", &msg);
                anyhow::anyhow!("{}", msg)
            })?;

        // Get the handle function
        let globals = self.lua.globals();
        let handle_fn: Value = globals
            .get("handle")
            .map_err(|e| anyhow::anyhow!("failed to get handle: {}", e))?;

        if handle_fn == Value::Nil {
            debug!("rule script '{}' does not define handle()", script_name);
            return Ok(None);
        }

        let func: mlua::Function = handle_fn
            .as_function()
            .ok_or_else(|| anyhow::anyhow!("handle is not a function"))?
            .clone();

        // Call handle(tick, state) where state is nil for now
        // TODO: pass room state table
        let result: Value = func.call((tick, Value::Nil)).map_err(|e| {
            let msg = format!("rule handle() failed for '{}': {}", script_name, e);
            record_lua_error("rule_handle", &msg);
            anyhow::anyhow!("{}", msg)
        })?;

        // Clean up the handle function so it doesn't pollute globals for next script
        globals.set("handle", Value::Nil)?;

        // Return table result if present (used for notify slots)
        match result {
            Value::Table(t) => Ok(Some(t)),
            _ => Ok(None),
        }
    }

    /// Check if the script defines a background() function
    pub fn has_background(&self) -> bool {
        self.lua
            .globals()
            .get::<Value>("background")
            .map(|v| v != Value::Nil)
            .unwrap_or(false)
    }

    /// Check if the script defines an on_tick() function
    pub fn has_on_tick(&self) -> bool {
        self.lua
            .globals()
            .get::<Value>("on_tick")
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
    use crate::llm::LlmClient;
    use crate::mcp::McpManager;
    use crate::model::{ModelBackend, ModelHandle, ModelRegistry};
    use crate::rules::RulesEngine;
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
                rules: Arc::new(RulesEngine::new()),
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
            use crate::db::rows::Row;

            // Get or create buffer
            if let Ok(buffer) = self.shared_state.db.get_or_create_room_buffer(room) {
                // Get or create agent
                if let Ok(agent) = self.shared_state.db.get_or_create_human_agent(sender) {
                    let mut row = Row::message(&buffer.id, &agent.id, content, false);
                    let _ = self.shared_state.db.append_row(&mut row);
                }
            }
        }
    }

    #[test]
    fn test_lua_runtime_new() {
        let runtime = LuaRuntime::new().expect("should create runtime");
        assert!(!runtime.has_user_script());
    }

    #[test]
    fn test_on_tick_default() {
        use std::sync::{Arc, Mutex};

        let runtime = LuaRuntime::new().expect("should create runtime");

        // Create a render buffer
        let render_buffer = Arc::new(Mutex::new(crate::ui::RenderBuffer::new(80, 8)));

        // Call on_tick - should draw to buffer without error
        // Lua queries status via sshwarma.call("status") - no pre-population needed
        runtime
            .call_on_tick(1, render_buffer.clone(), 80, 8)
            .expect("should call on_tick");

        // Get output - should have content
        let buf = render_buffer.lock().unwrap();
        let output = buf.to_ansi();

        // Embedded hud.lua should draw frame with box chars
        assert!(!output.is_empty(), "HUD should produce output");
    }

    #[test]
    fn test_on_tick_renders_to_buffer() {
        use std::sync::{Arc, Mutex};

        let runtime = LuaRuntime::new().expect("should create runtime");

        let render_buffer = Arc::new(Mutex::new(crate::ui::RenderBuffer::new(80, 8)));

        // Call on_tick multiple times (simulating timer)
        for tick in 1..5 {
            runtime
                .call_on_tick(tick, render_buffer.clone(), 80, 8)
                .expect("should call on_tick");
        }

        // Buffer should have content from drawing
        let buf = render_buffer.lock().unwrap();
        let output = buf.to_ansi();
        assert!(!output.is_empty(), "Buffer should have content after on_tick");
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

    // =========================================================================
    // Hot-reload tests
    // =========================================================================

    #[test]
    fn test_hot_reload_detects_file_change() {
        use std::sync::{Arc, Mutex};
        use std::thread;
        use std::time::Duration;

        // Create a temp file with initial script
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join(format!("test_hud_{}.lua", std::process::id()));

        let initial_script = r#"
test_value = "initial"
function on_tick(tick, ctx)
    ctx:clear()
    ctx:print(0, 0, test_value)
end
"#;

        fs::write(&script_path, initial_script).expect("should write initial script");

        // Create runtime and load the script
        let mut runtime = LuaRuntime::new().expect("should create runtime");
        runtime
            .load_script(&script_path)
            .expect("should load script");

        // Verify initial state
        assert!(runtime.has_user_script());
        assert!(!runtime.check_reload(), "should not reload when unchanged");

        // Wait a moment to ensure mtime will be different
        thread::sleep(Duration::from_millis(50));

        // Modify the file
        let modified_script = r#"
test_value = "modified"
function on_tick(tick, ctx)
    ctx:clear()
    ctx:print(0, 0, test_value)
end
"#;
        fs::write(&script_path, modified_script).expect("should write modified script");

        // Check reload should detect the change
        assert!(runtime.check_reload(), "should detect file modification");

        // Verify new script is loaded by calling on_tick
        let render_buffer = Arc::new(Mutex::new(crate::ui::RenderBuffer::new(80, 8)));
        runtime
            .call_on_tick(1, render_buffer.clone(), 80, 8)
            .expect("should call on_tick");

        // Check that buffer contains "modified"
        let buf = render_buffer.lock().unwrap();
        let output = buf.to_ansi();
        assert!(output.contains("modified"), "should use modified script");

        // Cleanup
        let _ = fs::remove_file(&script_path);
    }

    #[test]
    fn test_hot_reload_fallback_on_delete() {
        use std::sync::{Arc, Mutex};

        // Create a temp file
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join(format!("test_hud_delete_{}.lua", std::process::id()));

        let script = r#"
function on_tick(tick, ctx)
    ctx:clear()
    ctx:print(0, 0, "user script")
end
"#;
        fs::write(&script_path, script).expect("should write script");

        // Load the script
        let mut runtime = LuaRuntime::new().expect("should create runtime");
        runtime
            .load_script(&script_path)
            .expect("should load script");
        assert!(runtime.has_user_script());

        // Delete the file
        fs::remove_file(&script_path).expect("should delete file");

        // Check reload should fallback to embedded
        assert!(runtime.check_reload(), "should detect file deletion");
        assert!(
            !runtime.has_user_script(),
            "should fallback to embedded script"
        );

        // Should still render (using embedded default)
        let render_buffer = Arc::new(Mutex::new(crate::ui::RenderBuffer::new(80, 8)));
        runtime
            .call_on_tick(1, render_buffer.clone(), 80, 8)
            .expect("should call on_tick with embedded script");

        let buf = render_buffer.lock().unwrap();
        let output = buf.to_ansi();
        assert!(!output.is_empty(), "should produce output");
    }

    #[test]
    fn test_hot_reload_keeps_previous_on_syntax_error() {
        use std::sync::{Arc, Mutex};
        use std::thread;
        use std::time::Duration;

        // Create a temp file with valid script
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join(format!("test_hud_syntax_{}.lua", std::process::id()));

        let valid_script = r#"
function on_tick(tick, ctx)
    ctx:clear()
    ctx:print(0, 0, "valid")
end
"#;
        fs::write(&script_path, valid_script).expect("should write valid script");

        let mut runtime = LuaRuntime::new().expect("should create runtime");
        runtime
            .load_script(&script_path)
            .expect("should load valid script");

        // Wait to ensure mtime changes
        thread::sleep(Duration::from_millis(50));

        // Write invalid script
        let invalid_script = r#"
function on_tick(tick, ctx
    -- Missing closing paren = syntax error
    ctx:clear()
end
"#;
        fs::write(&script_path, invalid_script).expect("should write invalid script");

        // Check reload - should attempt but fail, keeping previous version
        let _reloaded = runtime.check_reload();
        // The reload attempt happened but may have failed
        // The important thing is that on_tick still works

        let render_buffer = Arc::new(Mutex::new(crate::ui::RenderBuffer::new(80, 8)));
        let result = runtime.call_on_tick(1, render_buffer.clone(), 80, 8);

        // Should still be able to render (using previous valid script)
        assert!(result.is_ok(), "should still render after failed reload");

        // Cleanup
        let _ = fs::remove_file(&script_path);
    }
}
