//! Lua integration for sshwarma
//!
//! This module provides Lua scripting support for customizable screen rendering.
//! Supports hot-reloading of user scripts from ~/.config/sshwarma/screen.lua.

pub mod cache;
pub mod context;
pub mod data;
pub mod dirty;
pub mod mcp_bridge;
pub mod registry;
pub mod render;
pub mod tool_middleware;
pub mod tools;
pub mod wrap;

pub use cache::ToolCache;
pub use context::{NotificationLevel, PendingNotification};
pub use dirty::DirtyState;
pub use mcp_bridge::{mcp_request_handler, McpBridge};
pub use registry::ToolRegistry;
pub use render::parse_lua_rows;
pub use tool_middleware::{ToolContext, ToolMiddleware};
pub use tools::{register_mcp_tools, InputState, LuaToolState, SessionContext};
pub use wrap::{WrapResult, WrapState};

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

/// Embedded default screen script (full UI: chat, status, input)
const DEFAULT_SCREEN_SCRIPT: &str = include_str!("../embedded/screen.lua");

/// Embedded wrap script for context composition
const DEFAULT_WRAP_SCRIPT: &str = include_str!("../embedded/wrap.lua");

/// Embedded input module for raw byte handling and escape sequence parsing
const INPUT_MODULE: &str = include_str!("../embedded/ui/input.lua");

/// Embedded layout module for constraint solving
const LAYOUT_MODULE: &str = include_str!("../embedded/ui/layout.lua");

/// Embedded bars module for edge chrome
const BARS_MODULE: &str = include_str!("../embedded/ui/bars.lua");

/// Embedded pages module for page stack management
const PAGES_MODULE: &str = include_str!("../embedded/ui/pages.lua");

/// Embedded scroll module for scroll state
const SCROLL_MODULE: &str = include_str!("../embedded/ui/scroll.lua");

/// Embedded mode module for vim-style modes
const MODE_MODULE: &str = include_str!("../embedded/ui/mode.lua");

/// Embedded bootstrap script for custom require system
const BOOTSTRAP_SCRIPT: &str = include_str!("../embedded/init.lua");

/// Embedded inspect.lua library
const INSPECT_MODULE: &str = include_str!("../embedded/lib/inspect.lua");

/// Embedded luafun functional programming library (MIT license)
const FUN_MODULE: &str = include_str!("../embedded/lib/fun.lua");

/// Embedded str.lua string utilities (MIT license)
const STR_MODULE: &str = include_str!("../embedded/lib/str.lua");

/// Page helper for commands - opens pages directly
const PAGE_MODULE: &str = include_str!("../embedded/lib/page.lua");

/// Help documentation - luafun quick reference
const HELP_FUN: &str = include_str!("../embedded/help/fun.md");

/// Help documentation - str.lua quick reference
const HELP_STR: &str = include_str!("../embedded/help/str.md");

/// Help documentation - inspect.lua quick reference
const HELP_INSPECT: &str = include_str!("../embedded/help/inspect.md");

/// Help documentation - MCP tools quick reference
const HELP_TOOLS: &str = include_str!("../embedded/help/tools.md");

/// Help documentation - Room navigation and vibes
const HELP_ROOM: &str = include_str!("../embedded/help/room.md");

/// Help documentation - Journal entries and decisions
const HELP_JOURNAL: &str = include_str!("../embedded/help/journal.md");

/// Help system module
const HELP_MODULE: &str = include_str!("../embedded/lib/help.lua");

/// Embedded commands dispatcher
const COMMANDS_MODULE: &str = include_str!("../embedded/commands/init.lua");

/// Embedded navigation commands
const COMMANDS_NAV_MODULE: &str = include_str!("../embedded/commands/nav.lua");

/// Embedded room management commands
const COMMANDS_ROOM_MODULE: &str = include_str!("../embedded/commands/room.lua");

/// Embedded inventory commands
const COMMANDS_INVENTORY_MODULE: &str = include_str!("../embedded/commands/inventory.lua");

/// Embedded journal commands
const COMMANDS_JOURNAL_MODULE: &str = include_str!("../embedded/commands/journal.lua");

/// Embedded MCP commands
const COMMANDS_MCP_MODULE: &str = include_str!("../embedded/commands/mcp.lua");

/// Embedded history commands
const COMMANDS_HISTORY_MODULE: &str = include_str!("../embedded/commands/history.lua");

/// Embedded debug commands
const COMMANDS_DEBUG_MODULE: &str = include_str!("../embedded/commands/debug.lua");

/// Embedded prompt commands
const COMMANDS_PROMPT_MODULE: &str = include_str!("../embedded/commands/prompt.lua");

/// Embedded rules commands
const COMMANDS_RULES_MODULE: &str = include_str!("../embedded/commands/rules.lua");

/// Embedded reload commands
const COMMANDS_RELOAD_MODULE: &str = include_str!("../embedded/commands/reload.lua");

/// Registry of embedded Lua modules
///
/// Provides module lookup for the custom require system.
/// Modules are included at compile time via include_str!.
pub struct EmbeddedModules {
    modules: std::collections::HashMap<String, &'static str>,
}

impl EmbeddedModules {
    /// Create a new registry with all embedded modules
    pub fn new() -> Self {
        let mut modules = std::collections::HashMap::new();

        // Library modules
        modules.insert("inspect".to_string(), INSPECT_MODULE);
        modules.insert("fun".to_string(), FUN_MODULE);
        modules.insert("str".to_string(), STR_MODULE);
        modules.insert("page".to_string(), PAGE_MODULE);

        // Help documentation modules
        modules.insert("help.fun".to_string(), HELP_FUN);
        modules.insert("help.str".to_string(), HELP_STR);
        modules.insert("help.inspect".to_string(), HELP_INSPECT);
        modules.insert("help.tools".to_string(), HELP_TOOLS);
        modules.insert("help.room".to_string(), HELP_ROOM);
        modules.insert("help.journal".to_string(), HELP_JOURNAL);

        // Help system module
        modules.insert("help".to_string(), HELP_MODULE);

        // Core modules
        modules.insert("screen".to_string(), DEFAULT_SCREEN_SCRIPT);

        // UI modules
        modules.insert("ui.input".to_string(), INPUT_MODULE);
        modules.insert("ui.layout".to_string(), LAYOUT_MODULE);
        modules.insert("ui.bars".to_string(), BARS_MODULE);
        modules.insert("ui.pages".to_string(), PAGES_MODULE);
        modules.insert("ui.scroll".to_string(), SCROLL_MODULE);
        modules.insert("ui.mode".to_string(), MODE_MODULE);

        // Command modules
        modules.insert("commands".to_string(), COMMANDS_MODULE);
        modules.insert("commands.nav".to_string(), COMMANDS_NAV_MODULE);
        modules.insert("commands.room".to_string(), COMMANDS_ROOM_MODULE);
        modules.insert("commands.inventory".to_string(), COMMANDS_INVENTORY_MODULE);
        modules.insert("commands.journal".to_string(), COMMANDS_JOURNAL_MODULE);
        modules.insert("commands.mcp".to_string(), COMMANDS_MCP_MODULE);
        modules.insert("commands.history".to_string(), COMMANDS_HISTORY_MODULE);
        modules.insert("commands.debug".to_string(), COMMANDS_DEBUG_MODULE);
        modules.insert("commands.prompt".to_string(), COMMANDS_PROMPT_MODULE);
        modules.insert("commands.rules".to_string(), COMMANDS_RULES_MODULE);
        modules.insert("commands.reload".to_string(), COMMANDS_RELOAD_MODULE);

        Self { modules }
    }

    /// Get an embedded module by name
    ///
    /// Module names use dot notation: "ui.input", "commands.nav"
    pub fn get(&self, name: &str) -> Option<&'static str> {
        self.modules.get(name).copied()
    }

    /// List all embedded module names (for debugging)
    pub fn list(&self) -> Vec<&str> {
        self.modules.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for EmbeddedModules {
    fn default() -> Self {
        Self::new()
    }
}

/// Register embedded modules and sshwarma table for custom require
///
/// Sets up the `sshwarma` global table with:
/// - `sshwarma.config_path`: User's config directory
/// - `sshwarma.get_embedded_module(name)`: Get embedded module source
/// - `sshwarma.list_embedded_modules()`: List all embedded modules
fn register_embedded_modules(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();

    // Get or create sshwarma table (may already exist from register_sshwarma_call)
    let sshwarma: Table = match globals.get::<Value>("sshwarma")? {
        Value::Table(t) => t,
        _ => {
            let t = lua.create_table()?;
            globals.set("sshwarma", t.clone())?;
            t
        }
    };

    // Set config_path
    let config_path = paths::config_dir();
    sshwarma.set("config_path", config_path.to_string_lossy().to_string())?;

    // Create embedded modules registry
    let embedded = EmbeddedModules::new();

    // get_embedded_module(name) -> string or nil
    let modules_for_get = embedded.modules.clone();
    let get_embedded =
        lua.create_function(move |_, modname: String| Ok(modules_for_get.get(&modname).copied()))?;
    sshwarma.set("get_embedded_module", get_embedded)?;

    // list_embedded_modules() -> table of names
    let modules_for_list = embedded.modules.clone();
    let list_embedded = lua.create_function(move |lua, ()| {
        let names = lua.create_table()?;
        for (i, name) in modules_for_list.keys().enumerate() {
            names.set(i + 1, name.as_str())?;
        }
        Ok(names)
    })?;
    sshwarma.set("list_embedded_modules", list_embedded)?;

    Ok(())
}

/// Run the bootstrap script to set up custom require system
///
/// This installs a custom searcher in package.searchers that:
/// 1. Checks user config directory (~/.config/sshwarma/lua/)
/// 2. Checks embedded modules
/// 3. Falls through to standard package.path
fn run_bootstrap(lua: &Lua) -> mlua::Result<()> {
    lua.load(BOOTSTRAP_SCRIPT)
        .set_name("embedded:init.lua")
        .exec()
}

/// Path to user's custom screen script
pub fn user_screen_script_path() -> PathBuf {
    paths::config_dir().join("screen.lua")
}

/// Path to a specific user's screen script (e.g., atobey.lua, claude.lua)
pub fn user_named_script_path(username: &str) -> PathBuf {
    paths::config_dir().join(format!("{}.lua", username))
}

/// Path to server startup script
pub fn startup_script_path() -> PathBuf {
    paths::config_dir().join("startup.lua")
}

/// Lua runtime for screen rendering
///
/// Manages the Lua state, script loading, and hot-reloading.
/// Lua scripts implement `on_tick(tick, ctx)` to render the screen.
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

        // Register embedded modules for custom require system
        register_embedded_modules(&lua)
            .map_err(|e| anyhow::anyhow!("failed to register embedded modules: {}", e))?;

        // Run bootstrap to install custom searcher in package.searchers
        run_bootstrap(&lua).map_err(|e| anyhow::anyhow!("failed to run Lua bootstrap: {}", e))?;

        // Get package.loaded for registering modules
        let package: Table = lua.globals().get("package")?;
        let loaded: Table = package.get("loaded")?;

        // Load the input module (provides escape sequence parsing, input buffer)
        lua.load(INPUT_MODULE)
            .set_name("embedded:ui/input.lua")
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to load embedded input module: {}", e))?;

        // Get the module table that was set as global 'input'
        let input_chunk: Table = lua
            .globals()
            .get("input")
            .map_err(|e| anyhow::anyhow!("input module not set globally: {}", e))?;
        loaded.set("ui.input", input_chunk)?;

        // Load the inspect module (for debugging, table formatting)
        let inspect_chunk = lua
            .load(INSPECT_MODULE)
            .set_name("embedded:lib/inspect.lua")
            .eval::<Table>()
            .map_err(|e| anyhow::anyhow!("failed to load embedded inspect module: {}", e))?;

        loaded.set("inspect", inspect_chunk.clone())?;
        lua.globals().set("inspect", inspect_chunk)?;

        // Load utility library modules (required by wrap and other modules)
        // fun.lua - functional programming library (must be loaded before wrap.lua)
        let fun_chunk = lua
            .load(FUN_MODULE)
            .set_name("embedded:lib/fun.lua")
            .eval::<Table>()
            .map_err(|e| anyhow::anyhow!("failed to load fun module: {}", e))?;
        loaded.set("fun", fun_chunk)?;

        // Load the wrap script (provides wrap() and default_wrap())
        // Depends on: fun
        lua.load(DEFAULT_WRAP_SCRIPT)
            .set_name("embedded:wrap.lua")
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to load embedded wrap script: {}", e))?;

        // str.lua - string utilities (required by help.lua)
        let str_chunk = lua
            .load(STR_MODULE)
            .set_name("embedded:lib/str.lua")
            .eval::<Table>()
            .map_err(|e| anyhow::anyhow!("failed to load str module: {}", e))?;
        loaded.set("str", str_chunk)?;

        // Load new UI modules (require fun to be loaded first)
        let layout_chunk = lua
            .load(LAYOUT_MODULE)
            .set_name("embedded:ui/layout.lua")
            .eval::<Table>()
            .map_err(|e| anyhow::anyhow!("failed to load embedded layout module: {}", e))?;
        loaded.set("ui.layout", layout_chunk)?;

        let bars_chunk = lua
            .load(BARS_MODULE)
            .set_name("embedded:ui/bars.lua")
            .eval::<Table>()
            .map_err(|e| anyhow::anyhow!("failed to load embedded bars module: {}", e))?;
        loaded.set("ui.bars", bars_chunk)?;

        let pages_chunk = lua
            .load(PAGES_MODULE)
            .set_name("embedded:ui/pages.lua")
            .eval::<Table>()
            .map_err(|e| anyhow::anyhow!("failed to load embedded pages module: {}", e))?;
        loaded.set("ui.pages", pages_chunk)?;

        let scroll_chunk = lua
            .load(SCROLL_MODULE)
            .set_name("embedded:ui/scroll.lua")
            .eval::<Table>()
            .map_err(|e| anyhow::anyhow!("failed to load embedded scroll module: {}", e))?;
        loaded.set("ui.scroll", scroll_chunk)?;

        // Load mode module - defines on_input() globally
        let mode_chunk = lua
            .load(MODE_MODULE)
            .set_name("embedded:ui/mode.lua")
            .eval::<Table>()
            .map_err(|e| anyhow::anyhow!("failed to load embedded mode module: {}", e))?;
        loaded.set("ui.mode", mode_chunk)?;

        // page.lua - helper for commands to open pages (requires ui.pages)
        let page_chunk = lua
            .load(PAGE_MODULE)
            .set_name("embedded:lib/page.lua")
            .eval::<Table>()
            .map_err(|e| anyhow::anyhow!("failed to load page module: {}", e))?;
        loaded.set("page", page_chunk)?;

        // help.lua - help system (required by commands/init.lua)
        let help_chunk = lua
            .load(HELP_MODULE)
            .set_name("embedded:lib/help.lua")
            .eval::<Table>()
            .map_err(|e| anyhow::anyhow!("failed to load help module: {}", e))?;
        loaded.set("help", help_chunk)?;

        // Pre-load all command modules into package.loaded
        // This avoids needing load() which isn't available in Luau sandbox
        let cmd_modules: &[(&str, &str, &str)] = &[
            (
                "commands.nav",
                COMMANDS_NAV_MODULE,
                "embedded:commands/nav.lua",
            ),
            (
                "commands.room",
                COMMANDS_ROOM_MODULE,
                "embedded:commands/room.lua",
            ),
            (
                "commands.inventory",
                COMMANDS_INVENTORY_MODULE,
                "embedded:commands/inventory.lua",
            ),
            (
                "commands.journal",
                COMMANDS_JOURNAL_MODULE,
                "embedded:commands/journal.lua",
            ),
            (
                "commands.mcp",
                COMMANDS_MCP_MODULE,
                "embedded:commands/mcp.lua",
            ),
            (
                "commands.history",
                COMMANDS_HISTORY_MODULE,
                "embedded:commands/history.lua",
            ),
            (
                "commands.debug",
                COMMANDS_DEBUG_MODULE,
                "embedded:commands/debug.lua",
            ),
            (
                "commands.prompt",
                COMMANDS_PROMPT_MODULE,
                "embedded:commands/prompt.lua",
            ),
            (
                "commands.rules",
                COMMANDS_RULES_MODULE,
                "embedded:commands/rules.lua",
            ),
            (
                "commands.reload",
                COMMANDS_RELOAD_MODULE,
                "embedded:commands/reload.lua",
            ),
        ];

        for (name, code, chunk_name) in cmd_modules {
            let module: Table = lua
                .load(*code)
                .set_name(*chunk_name)
                .eval()
                .map_err(|e| anyhow::anyhow!("failed to load {}: {}", name, e))?;
            loaded.set(*name, module)?;
        }

        // Now load the main commands module (it will find submodules in package.loaded)
        let commands_module: Table = lua
            .load(COMMANDS_MODULE)
            .set_name("embedded:commands/init.lua")
            .eval()
            .map_err(|e| anyhow::anyhow!("failed to load commands module: {}", e))?;
        loaded.set("commands", commands_module.clone())?;
        lua.globals().set("commands", commands_module)?;

        // Load the default screen script
        lua.load(DEFAULT_SCREEN_SCRIPT)
            .set_name("embedded:screen.lua")
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to load embedded screen script: {}", e))?;

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
    /// 2. `screen.lua` (shared fallback)
    /// 3. Embedded default
    pub fn new_for_user(username: Option<&str>) -> Result<Self> {
        let mut runtime = Self::new()?;

        // Try user-specific script first (e.g., atobey.lua)
        if let Some(name) = username {
            let named_path = user_named_script_path(name);
            if named_path.exists() {
                match runtime.load_script(&named_path) {
                    Ok(()) => {
                        info!("Loaded screen script for '{}' from {:?}", name, named_path);
                        return Ok(runtime);
                    }
                    Err(e) => {
                        warn!(
                            "Failed to load screen script {:?}: {}. Trying fallback.",
                            named_path, e
                        );
                    }
                }
            }
        }

        // Try shared user script (screen.lua)
        let user_path = user_screen_script_path();
        if user_path.exists() {
            match runtime.load_script(&user_path) {
                Ok(()) => {
                    info!("Loaded user screen script from {:?}", user_path);
                }
                Err(e) => {
                    warn!(
                        "Failed to load user screen script {:?}: {}. Using embedded default.",
                        user_path, e
                    );
                }
            }
        } else {
            debug!(
                "No user screen script at {:?}, using embedded default",
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

        debug!("Loaded screen script from {:?}", path);
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
            .load(DEFAULT_SCREEN_SCRIPT)
            .set_name("embedded:screen.lua")
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to reload embedded screen script: {}", e))?;

        self.loaded_script_path = None;
        self.loaded_script_mtime = None;

        Ok(())
    }

    /// Try to load user's UI entrypoint from database
    ///
    /// If the user has a configured entrypoint module, loads it via require()
    /// and sets its on_tick as the global on_tick function.
    ///
    /// Returns Ok(true) if entrypoint was loaded, Ok(false) if no entrypoint configured.
    pub fn try_load_db_entrypoint(
        &self,
        db: &crate::db::Database,
        username: &str,
    ) -> Result<bool> {
        // Check if user has an entrypoint configured
        let entrypoint = match db.get_user_entrypoint(username)? {
            Some(ep) => ep,
            None => return Ok(false),
        };

        info!(
            "Loading user UI entrypoint '{}' for '{}'",
            entrypoint, username
        );

        // Load the module via require and extract on_tick
        // The searcher in init.lua will check user DB first, then embedded
        let load_code = format!(
            r#"
            local ok, m = pcall(require, '{}')
            if ok and type(m) == 'table' and type(m.on_tick) == 'function' then
                on_tick = m.on_tick
                return true
            elseif ok and type(m) == 'table' then
                -- Module loaded but no on_tick
                return false, 'module does not export on_tick function'
            else
                -- require failed
                return false, tostring(m)
            end
            "#,
            entrypoint
        );

        let result: (bool, Option<String>) = self
            .lua
            .load(&load_code)
            .set_name("@entrypoint_loader")
            .eval()
            .map_err(|e| anyhow::anyhow!("failed to load entrypoint '{}': {}", entrypoint, e))?;

        match result {
            (true, _) => {
                info!("Successfully loaded UI entrypoint '{}'", entrypoint);
                Ok(true)
            }
            (false, Some(err)) => {
                warn!("Failed to load UI entrypoint '{}': {}", entrypoint, err);
                Err(anyhow::anyhow!(
                    "failed to load entrypoint '{}': {}",
                    entrypoint,
                    err
                ))
            }
            (false, None) => {
                warn!(
                    "UI entrypoint '{}' does not export on_tick function",
                    entrypoint
                );
                Ok(false)
            }
        }
    }

    /// Reload UI from database or embedded default
    ///
    /// Clears package.loaded cache and reloads the user's entrypoint from DB,
    /// or falls back to embedded default if no entrypoint is configured.
    pub fn reload_ui(&mut self, db: &crate::db::Database, username: &str) -> Result<()> {
        // Clear cached modules to force reload
        let clear_cache = r#"
            if package and package.loaded then
                -- Clear user/room modules but keep system modules
                for k in pairs(package.loaded) do
                    if not k:match('^sshwarma%.') and
                       not k:match('^commands%.') and
                       not k:match('^ui%.') and
                       k ~= 'inspect' then
                        package.loaded[k] = nil
                    end
                end
            end
        "#;

        self.lua
            .load(clear_cache)
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to clear module cache: {}", e))?;

        // Try to load user entrypoint from DB
        match self.try_load_db_entrypoint(db, username) {
            Ok(true) => {
                info!("Reloaded UI from database entrypoint");
                self.loaded_script_path = None; // Mark as DB-loaded, not file-loaded
            }
            Ok(false) | Err(_) => {
                // Fall back to embedded default
                info!("Reloading embedded default UI");
                self.reload_embedded()?;
            }
        }

        Ok(())
    }

    /// Reset to embedded default UI (clear user entrypoint)
    pub fn reset_to_default(&mut self) -> Result<()> {
        // Clear cached modules
        let clear_cache = r#"
            if package and package.loaded then
                for k in pairs(package.loaded) do
                    if not k:match('^sshwarma%.') and
                       not k:match('^commands%.') and
                       not k:match('^ui%.') and
                       k ~= 'inspect' then
                        package.loaded[k] = nil
                    end
                end
            end
        "#;

        self.lua
            .load(clear_cache)
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to clear module cache: {}", e))?;

        self.reload_embedded()?;
        info!("Reset to embedded default UI");
        Ok(())
    }

    /// Push a notification for the screen to display
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

    /// Build context for LLM interactions via Lua wrap() system
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
    pub fn wrap(&self, wrap_state: WrapState, target_tokens: usize) -> Result<WrapResult> {
        use crate::lua::tools::SessionContext;

        // Save current session context (don't clobber screen's context)
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

        // Save current session context (don't clobber screen's context)
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

        // Save current session context (don't clobber screen's context)
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

    /// Call on_tick for screen rendering (legacy API)
    ///
    /// This is a convenience wrapper that marks all regions dirty.
    /// For tag-based partial updates, use `call_on_tick_with_tags` instead.
    pub fn call_on_tick(
        &self,
        tick: u64,
        render_buffer: std::sync::Arc<std::sync::Mutex<crate::ui::RenderBuffer>>,
        width: u16,
        height: u16,
    ) -> Result<()> {
        // Mark all standard regions dirty for full render
        let mut all_dirty = std::collections::HashSet::new();
        all_dirty.insert("status".to_string());
        all_dirty.insert("chat".to_string());
        all_dirty.insert("input".to_string());

        self.call_on_tick_with_tags(&all_dirty, tick, render_buffer, width, height)
    }

    /// Call on_tick with dirty tags for tag-based partial updates
    ///
    /// The dirty_tags set is passed to Lua as a table where keys are tag names
    /// and values are true. Lua can check `if dirty_tags["chat"] then ... end`
    /// to decide which regions to render.
    ///
    /// This enables Lua to control layout: status at top, bottom, both sides,
    /// or mixed with content - Rust imposes no layout assumptions.
    pub fn call_on_tick_with_tags(
        &self,
        dirty_tags: &std::collections::HashSet<String>,
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

        // Convert dirty_tags to Lua table: {status = true, chat = true, ...}
        let tags_table = self
            .lua
            .create_table()
            .map_err(|e| anyhow::anyhow!("failed to create tags table: {}", e))?;
        for tag in dirty_tags {
            tags_table
                .set(tag.as_str(), true)
                .map_err(|e| anyhow::anyhow!("failed to set tag {}: {}", tag, e))?;
        }

        // Create draw context for full screen
        let ctx = crate::ui::LuaDrawContext::new(render_buffer, 0, 0, width, height);

        // Call on_tick(dirty_tags, tick, ctx) - new signature
        // For backwards compatibility, also try (tick, ctx) if that fails
        let result = func.call::<()>((tags_table.clone(), tick, ctx.clone()));

        if let Err(e) = result {
            // Try old signature (tick, ctx) for backwards compatibility
            let old_result = func.call::<()>((tick, ctx));
            if let Err(old_e) = old_result {
                let msg = format!(
                    "on_tick() call failed: {} (also tried old signature: {})",
                    e, old_e
                );
                record_lua_error("on_tick", &msg);
                return Err(anyhow::anyhow!("{}", msg));
            }
        }

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

    /// Check if the script defines an on_input() function
    pub fn has_on_input(&self) -> bool {
        self.lua
            .globals()
            .get::<Value>("on_input")
            .map(|v| v != Value::Nil)
            .unwrap_or(false)
    }

    /// Forward raw input bytes to Lua's on_input() function
    ///
    /// Lua parses escape sequences and manages the input buffer.
    /// Returns an action table describing what Rust should do (execute, tab, quit, etc.)
    ///
    /// # Returns
    /// - `Ok(Some(action))` - Action table with `type` field
    /// - `Ok(None)` - No action needed (just redraw)
    /// - `Err(_)` - Lua error
    pub fn call_on_input(&self, bytes: &[u8]) -> Result<Option<InputAction>> {
        let globals = self.lua.globals();

        // Check if on_input function exists
        let on_input_fn: Value = globals
            .get("on_input")
            .map_err(|e| anyhow::anyhow!("failed to get on_input: {}", e))?;

        if on_input_fn == Value::Nil {
            // No on_input function defined - this is unexpected but handle gracefully
            debug!("on_input function not defined");
            return Ok(None);
        }

        let func: mlua::Function = on_input_fn
            .as_function()
            .ok_or_else(|| anyhow::anyhow!("on_input is not a function"))?
            .clone();

        // Convert bytes to Lua string
        let lua_bytes = self
            .lua
            .create_string(bytes)
            .map_err(|e| anyhow::anyhow!("failed to create Lua string: {}", e))?;

        // Call on_input(bytes) -> action table or nil
        let result: Value = func.call(lua_bytes).map_err(|e| {
            let msg = format!("on_input() call failed: {}", e);
            record_lua_error("on_input", &msg);
            anyhow::anyhow!("{}", msg)
        })?;

        // Parse the action table
        match result {
            Value::Nil => Ok(None),
            Value::Table(t) => {
                let action_type: String = t
                    .get("type")
                    .map_err(|e| anyhow::anyhow!("action missing 'type': {}", e))?;

                let action = match action_type.as_str() {
                    "none" => InputAction::None,
                    "redraw" => InputAction::Redraw,
                    "execute" | "send" => {
                        let text: String = t
                            .get("text")
                            .map_err(|e| anyhow::anyhow!("execute/send action missing 'text': {}", e))?;
                        InputAction::Execute(text)
                    }
                    "tab" => InputAction::Tab,
                    "clear_screen" => InputAction::ClearScreen,
                    "quit" => InputAction::Quit,
                    "escape" => InputAction::Escape,
                    "page_up" => InputAction::PageUp,
                    "page_down" => InputAction::PageDown,
                    _ => {
                        warn!("unknown input action type: {}", action_type);
                        InputAction::None
                    }
                };

                Ok(Some(action))
            }
            _ => {
                warn!("on_input returned unexpected type: {:?}", result);
                Ok(None)
            }
        }
    }

    /// Dispatch a command through Lua's command system
    ///
    /// Loads the commands module and calls `commands.dispatch(name, args)`.
    /// Returns the result table converted to CommandResult.
    pub fn call_dispatch_command(&self, name: &str, args: &str) -> Result<Option<CommandResult>> {
        tracing::info!("call_dispatch_command: getting commands module");
        // Use require() to get cached commands module (or load if first time)
        let commands: Table = self
            .lua
            .load(
                r#"
                return require('commands')
            "#,
            )
            .eval()
            .map_err(|e| anyhow::anyhow!("failed to get commands module: {}", e))?;
        tracing::info!("call_dispatch_command: got commands module");

        // Check if dispatch function exists
        let dispatch_fn: mlua::Function = commands
            .get("dispatch")
            .map_err(|e| anyhow::anyhow!("commands.dispatch not found: {}", e))?;
        tracing::info!(
            "call_dispatch_command: calling dispatch({}, {})",
            name,
            args
        );

        // Call dispatch(name, args)
        let result: Table = dispatch_fn
            .call::<Table>((name, args))
            .map_err(|e| anyhow::anyhow!("commands.dispatch failed: {}", e))?;
        tracing::info!("call_dispatch_command: dispatch returned");

        // Parse result table {text, mode, title?}
        let text: String = result.get("text").unwrap_or_default();
        let mode: String = result
            .get("mode")
            .unwrap_or_else(|_| "notification".to_string());
        let title: Option<String> = result.get("title").ok();

        Ok(Some(CommandResult { text, mode, title }))
    }
}

/// Action returned by Lua input handler
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    /// No action needed
    None,
    /// Redraw the input line
    Redraw,
    /// Execute the given line
    Execute(String),
    /// Tab completion requested
    Tab,
    /// Clear screen requested
    ClearScreen,
    /// Quit (Ctrl+D on empty line)
    Quit,
    /// Escape key pressed (dismiss overlay, cancel, etc.)
    Escape,
    /// Scroll up one page
    PageUp,
    /// Scroll down one page
    PageDown,
}

/// Result from Lua command dispatch
#[derive(Debug, Clone)]
pub struct CommandResult {
    /// Output text to display
    pub text: String,
    /// Display mode: "overlay" or "notification"
    pub mode: String,
    /// Optional title for overlay mode
    pub title: Option<String>,
}

impl Default for CommandResult {
    fn default() -> Self {
        Self {
            text: String::new(),
            mode: "notification".to_string(),
            title: None,
        }
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

        // Embedded screen.lua should produce output
        assert!(!output.is_empty(), "screen should produce output");
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
        assert!(
            !output.is_empty(),
            "Buffer should have content after on_tick"
        );
    }

    #[test]
    fn test_notification_queue() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Push a notification
        runtime.push_notification("Test notification".to_string(), 5000);

        // The notification will be processed on next render
    }

    #[test]
    fn test_screen_refresh_initial_render() {
        use std::sync::{Arc, Mutex};

        let runtime = LuaRuntime::new().expect("should create runtime");

        // Simulate screen refresh: mark all regions dirty FIRST (as screen.rs does)
        let dirty = runtime.tool_state().dirty();
        dirty.mark_many(["status", "chat", "input"]);

        // Take dirty tags (as screen refresh loop does after waiting)
        let dirty_tags = dirty.take();

        // Should have all 3 tags
        assert!(
            dirty_tags.contains("status"),
            "should have status tag marked dirty"
        );
        assert!(
            dirty_tags.contains("chat"),
            "should have chat tag marked dirty"
        );
        assert!(
            dirty_tags.contains("input"),
            "should have input tag marked dirty"
        );

        // Now call on_tick_with_tags (as render_screen_with_tags does)
        let render_buffer = Arc::new(Mutex::new(crate::ui::RenderBuffer::new(80, 24)));
        runtime
            .call_on_tick_with_tags(&dirty_tags, 1, render_buffer.clone(), 80, 24)
            .expect("should call on_tick_with_tags");

        // Check that output was produced
        let buf = render_buffer.lock().unwrap();
        let output = buf.to_ansi();
        assert!(
            !output.is_empty(),
            "screen should produce output on initial render"
        );
    }

    #[test]
    fn test_on_input_exists() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Verify on_input function is globally available
        let globals = runtime.lua.globals();
        let on_input: mlua::Value = globals.get("on_input").expect("should get on_input");

        assert!(
            on_input != mlua::Value::Nil,
            "on_input should be defined as a global function"
        );
        assert!(on_input.is_function(), "on_input should be a function");
    }

    #[test]
    fn test_user_config_path() {
        let path = user_screen_script_path();
        assert!(path.to_string_lossy().contains("sshwarma"));
        assert!(path.to_string_lossy().ends_with("screen.lua"));
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
    // Custom require system tests
    // =========================================================================

    #[test]
    fn test_embedded_modules_registry() {
        let modules = EmbeddedModules::new();

        // Should have inspect module
        assert!(
            modules.get("inspect").is_some(),
            "inspect module should exist"
        );

        // Should have ui.input module
        assert!(
            modules.get("ui.input").is_some(),
            "ui.input module should exist"
        );

        // List should include all modules
        let list = modules.list();
        assert!(list.len() >= 3, "should have at least 3 modules");
    }

    #[test]
    fn test_inspect_available() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // inspect is loaded as a global by LuaRuntime::new()
        let result: bool = runtime
            .lua
            .load(
                r#"
                return type(inspect) == 'table' and type(inspect.inspect) == 'function'
            "#,
            )
            .eval()
            .expect("inspect global should be available");

        assert!(result, "inspect should be a table with inspect function");
    }

    #[test]
    fn test_inspect_functionality() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Test that inspect actually formats tables
        let result: String = runtime
            .lua
            .load(
                r#"
                return inspect({a = 1, b = "hello"})
            "#,
            )
            .eval()
            .expect("inspect should format table");

        assert!(result.contains("a = 1"), "should contain 'a = 1'");
        assert!(
            result.contains("b = \"hello\""),
            "should contain 'b = \"hello\"'"
        );
    }

    #[test]
    fn test_sshwarma_config_path() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // sshwarma.config_path should be set
        let result: String = runtime
            .lua
            .load(r#"return sshwarma.config_path"#)
            .eval()
            .expect("sshwarma.config_path should be set");

        assert!(
            result.contains("sshwarma"),
            "config_path should contain 'sshwarma'"
        );
    }

    #[test]
    fn test_sshwarma_list_embedded_modules() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // sshwarma.list_embedded_modules() should return module names
        let count: i64 = runtime
            .lua
            .load(
                r#"
                local modules = sshwarma.list_embedded_modules()
                return #modules
            "#,
            )
            .eval()
            .expect("list_embedded_modules should work");

        assert!(
            count >= 3,
            "should list at least 3 embedded modules, got {}",
            count
        );
    }

    #[test]
    fn test_sshwarma_get_embedded_module() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // sshwarma.get_embedded_module('inspect') should return source code
        let result: bool = runtime
            .lua
            .load(
                r#"
                local code = sshwarma.get_embedded_module('inspect')
                return type(code) == 'string' and #code > 100
            "#,
            )
            .eval()
            .expect("get_embedded_module should work");

        assert!(
            result,
            "get_embedded_module('inspect') should return source code"
        );
    }

    #[test]
    fn test_require_help_module() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // require 'help' should work and return a table with help() function
        let result: bool = runtime
            .lua
            .load(
                r#"
                local help = require('help')
                return type(help) == 'table' and type(help.help) == 'function'
            "#,
            )
            .eval()
            .expect("require('help') should work");

        assert!(result, "help module should be a table with help() function");
    }

    #[test]
    fn test_help_module_returns_content() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // help.help('fun') should return markdown content
        let result: bool = runtime
            .lua
            .load(
                r#"
                local help = require('help')
                local content = help.help('fun')
                return type(content) == 'string' and content:find('Lua Fun') ~= nil
            "#,
            )
            .eval()
            .expect("help.help('fun') should return content");

        assert!(result, "help('fun') should return Lua Fun documentation");
    }

    #[test]
    fn test_input_cursor_movement() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Test that cursor movement works correctly with 0-based cursor
        // Bug: prev_utf8_start expects 1-indexed position but cursor is 0-indexed
        let result: bool = runtime
            .lua
            .load(
                r#"
                local input = require 'ui.input'

                -- Start fresh
                input.clear()

                -- Insert "ab"
                input.insert("a")
                input.insert("b")

                local state = input.get_state()
                assert(state.text == "ab", "text should be 'ab', got: " .. state.text)
                assert(state.cursor == 2, "cursor should be 2 after inserting 'ab', got: " .. state.cursor)

                -- Move left once - should go from cursor=2 to cursor=1
                input.left()
                state = input.get_state()
                assert(state.cursor == 1, "cursor should be 1 after left(), got: " .. state.cursor)

                -- Move left again - should go from cursor=1 to cursor=0
                input.left()
                state = input.get_state()
                assert(state.cursor == 0, "cursor should be 0 after second left(), got: " .. state.cursor)

                -- Move right - should go back to cursor=1
                input.right()
                state = input.get_state()
                assert(state.cursor == 1, "cursor should be 1 after right(), got: " .. state.cursor)

                return true
            "#,
            )
            .eval()
            .expect("input cursor movement test should pass");

        assert!(result, "cursor movement should work correctly");
    }

    #[test]
    fn test_input_backspace() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Test that backspace works correctly
        let result: bool = runtime
            .lua
            .load(
                r#"
                local input = require 'ui.input'

                -- Start fresh
                input.clear()

                -- Insert "ab"
                input.insert("a")
                input.insert("b")

                local state = input.get_state()
                assert(state.text == "ab", "text should be 'ab'")
                assert(state.cursor == 2, "cursor should be 2")

                -- Backspace at end - should delete 'b', leaving "a"
                input.backspace()
                state = input.get_state()
                assert(state.text == "a", "text should be 'a' after backspace, got: " .. state.text)
                assert(state.cursor == 1, "cursor should be 1 after backspace, got: " .. state.cursor)

                -- Backspace again - should delete 'a', leaving ""
                input.backspace()
                state = input.get_state()
                assert(state.text == "", "text should be '' after second backspace, got: " .. state.text)
                assert(state.cursor == 0, "cursor should be 0 after second backspace, got: " .. state.cursor)

                return true
            "#,
            )
            .eval()
            .expect("input backspace test should pass");

        assert!(result, "backspace should work correctly");
    }

    #[test]
    fn test_input_delete() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Test that delete key works correctly
        let result: bool = runtime
            .lua
            .load(
                r#"
                local input = require 'ui.input'

                -- Start fresh
                input.clear()

                -- Insert "ab"
                input.insert("a")
                input.insert("b")

                -- Move to start
                input.home()
                local state = input.get_state()
                assert(state.cursor == 0, "cursor should be 0 at home")

                -- Delete at start - should delete 'a', leaving "b"
                input.delete()
                state = input.get_state()
                assert(state.text == "b", "text should be 'b' after delete, got: " .. state.text)
                assert(state.cursor == 0, "cursor should still be 0 after delete")

                return true
            "#,
            )
            .eval()
            .expect("input delete test should pass");

        assert!(result, "delete should work correctly");
    }

    #[test]
    fn test_custom_searcher_or_embedded_fallback() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // In standard Lua, package.searchers has our custom searcher
        // In Luau, package.searchers is nil but sshwarma.get_embedded_module works
        let result: bool = runtime
            .lua
            .load(
                r#"
                -- Either custom searcher is installed OR embedded module fallback works
                local has_searchers = package.searchers ~= nil
                local has_embedded = sshwarma and sshwarma.get_embedded_module ~= nil
                return has_searchers or has_embedded
            "#,
            )
            .eval()
            .expect("should check module loading capability");

        assert!(
            result,
            "should have either custom searcher or embedded module fallback"
        );
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
            .wrap(wrap_state, 8000)
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
            .wrap(wrap_state, 8000)
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
        let result = runtime.wrap(wrap_state, 100);
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

    // =========================================================================
    // Screen module tests (pure function tests)
    // =========================================================================

    #[test]
    fn test_screen_wrap_text_basic() {
        let runtime = LuaRuntime::new().expect("should create runtime");
        let result: Vec<String> = runtime
            .lua
            .load(r#"return screen.wrap_text("hello world", 20)"#)
            .eval()
            .expect("should run wrap_text");

        assert_eq!(result, vec!["hello world"]);
    }

    #[test]
    fn test_screen_wrap_text_wraps_long_line() {
        let runtime = LuaRuntime::new().expect("should create runtime");
        let result: Vec<String> = runtime
            .lua
            .load(r#"return screen.wrap_text("hello world this is a test", 12)"#)
            .eval()
            .expect("should run wrap_text");

        // Should wrap at word boundaries
        assert!(result.len() > 1, "should wrap to multiple lines");
        for line in &result {
            assert!(line.len() <= 12, "line should fit in width: {}", line);
        }
    }

    #[test]
    fn test_screen_wrap_text_preserves_newlines() {
        let runtime = LuaRuntime::new().expect("should create runtime");
        let result: Vec<String> = runtime
            .lua
            .load(r#"return screen.wrap_text("line1\nline2\nline3", 50)"#)
            .eval()
            .expect("should run wrap_text");

        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "line1");
        assert_eq!(result[1], "line2");
        assert_eq!(result[2], "line3");
    }

    #[test]
    fn test_screen_wrap_text_empty() {
        let runtime = LuaRuntime::new().expect("should create runtime");
        let result: Vec<String> = runtime
            .lua
            .load(r#"return screen.wrap_text("", 20)"#)
            .eval()
            .expect("should run wrap_text");

        assert_eq!(result, vec![""]);
    }

    #[test]
    fn test_screen_build_display_lines_basic() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        let result: i64 = runtime
            .lua
            .load(
                r#"
                local messages = {
                    {author = "alice", content = "hello", is_model = false},
                    {author = "bob", content = "hi there", is_model = false},
                }
                return #screen.build_display_lines(messages, 80, "alice")
            "#,
            )
            .eval()
            .expect("should build display lines");

        assert_eq!(result, 2, "should have 2 display lines");
    }

    #[test]
    fn test_screen_build_display_lines_wrapping() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        let result: i64 = runtime
            .lua
            .load(
                r#"
                local long_content = string.rep("word ", 20)
                local messages = {{author = "alice", content = long_content, is_model = false}}
                return #screen.build_display_lines(messages, 40, "alice")
            "#,
            )
            .eval()
            .expect("should build display lines");

        assert!(result > 1, "long message should wrap to multiple lines");
    }

    #[test]
    fn test_screen_build_display_lines_author_only_on_first() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        let result: (bool, bool) = runtime
            .lua
            .load(
                r#"
                local long_content = string.rep("word ", 20)
                local messages = {{author = "alice", content = long_content, is_model = false}}
                local lines = screen.build_display_lines(messages, 40, "alice")
                return lines[1].author ~= nil, lines[2].author == nil
            "#,
            )
            .eval()
            .expect("should check author presence");

        assert!(result.0, "first line should have author");
        assert!(result.1, "second line should not have author");
    }

    #[test]
    fn test_screen_display_width_ascii() {
        let runtime = LuaRuntime::new().expect("should create runtime");
        let result: i64 = runtime
            .lua
            .load(r#"return screen.display_width("hello")"#)
            .eval()
            .expect("should get display width");

        assert_eq!(result, 5);
    }

    #[test]
    fn test_screen_display_width_empty() {
        let runtime = LuaRuntime::new().expect("should create runtime");
        let result: i64 = runtime
            .lua
            .load(r#"return screen.display_width("")"#)
            .eval()
            .expect("should get display width");

        assert_eq!(result, 0);
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn test_require_commands_module() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        println!("Testing require('commands')...");
        let result: Result<mlua::Table, _> = runtime.lua.load("return require('commands')").eval();

        match &result {
            Ok(t) => {
                println!("Commands loaded!");
                let has_dispatch: bool = t.contains_key("dispatch").unwrap_or(false);
                println!("Has dispatch: {}", has_dispatch);
                assert!(
                    has_dispatch,
                    "commands module should have dispatch function"
                );
            }
            Err(e) => {
                println!("Error loading commands: {}", e);
                panic!("Failed to load commands module: {}", e);
            }
        }
    }

    #[test]
    fn test_chat_region_visible() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Check that bars system is working
        let result: String = runtime
            .lua
            .load(
                r#"
            local output = {}
            local bars = require 'ui.bars'

            -- Check bars module
            table.insert(output, "bars type: " .. type(bars))

            -- Get defined bars
            local all_bars = bars.all()
            table.insert(output, "defined bars (" .. #all_bars .. "):")
            for _, b in ipairs(all_bars) do
                table.insert(output, "  " .. b.name .. ": position=" .. b.position .. " priority=" .. b.priority)
            end

            -- Compute layout for 80x24 terminal
            local layout = bars.compute_layout(80, 24, {})
            table.insert(output, "layout content: row=" .. (layout.content and layout.content.row or "nil") ..
                " height=" .. (layout.content and layout.content.height or "nil"))

            -- Check status bar
            if layout.status then
                table.insert(output, "status: row=" .. layout.status.row)
            end

            -- Check input bar
            if layout.input then
                table.insert(output, "input: row=" .. layout.input.row)
            end

            return table.concat(output, "\n")
        "#,
            )
            .eval()
            .expect("should run");

        println!("{}", result);

        // Should have bars defined
        assert!(
            result.contains("status:"),
            "status bar should be in layout: {}",
            result
        );
        assert!(
            result.contains("input:"),
            "input bar should be in layout: {}",
            result
        );
        assert!(
            result.contains("layout content:"),
            "content area should be computed: {}",
            result
        );
    }

    #[test]
    fn test_on_tick_draws_status_and_input() {
        use std::sync::{Arc, Mutex};

        let runtime = LuaRuntime::new().expect("should create runtime");

        // Create a render buffer
        let render_buffer = Arc::new(Mutex::new(crate::ui::RenderBuffer::new(80, 24)));

        // First call on_tick
        runtime
            .call_on_tick(1, render_buffer.clone(), 80, 24)
            .expect("should call on_tick");

        // Check the buffer content
        let buf = render_buffer.lock().unwrap();

        // Get raw output to see what was drawn
        let output = buf.to_ansi();
        println!("Buffer output length: {}", output.len());

        // Helper to extract a line
        fn get_line(buf: &crate::ui::RenderBuffer, y: u16) -> String {
            let mut line = String::new();
            for x in 0..buf.width() {
                if let Some(cell) = buf.get(x, y) {
                    line.push(cell.char);
                }
            }
            line
        }

        // Check if specific regions have content
        // The chat region is y=0 to y=21 (22 lines)
        // The status is y=22
        // The input is y=23

        // Extract status line (y=22) - should have room info
        let status_str = get_line(&buf, 22);
        println!("Status line: '{}'", status_str.trim());

        // Extract input line (y=23)
        let input_str = get_line(&buf, 23);
        println!("Input line: '{}'", input_str.trim());

        // Check first line of chat (y=0)
        let chat_str = get_line(&buf, 0);
        println!("First chat line: '{}'", chat_str.trim());

        // We expect status and input to have content
        assert!(
            !status_str.trim().is_empty(),
            "status line should have content"
        );
        assert!(
            !input_str.trim().is_empty(),
            "input line should have content (at least prompt)"
        );

        // Chat may be empty if no history (which is expected without room)
    }

    #[test]
    fn test_chat_region_no_debug_output() {
        use std::sync::{Arc, Mutex};

        let runtime = LuaRuntime::new().expect("should create runtime");
        let render_buffer = Arc::new(Mutex::new(crate::ui::RenderBuffer::new(80, 24)));

        // Call on_tick
        runtime
            .call_on_tick(1, render_buffer.clone(), 80, 24)
            .expect("should call on_tick");

        // Helper to get line content
        fn get_line(buf: &crate::ui::RenderBuffer, y: u16) -> String {
            let mut line = String::new();
            for x in 0..buf.width() {
                if let Some(cell) = buf.get(x, y) {
                    line.push(cell.char);
                }
            }
            line
        }

        let buf = render_buffer.lock().unwrap();

        // Check first few lines don't have debug markers
        // Debug output used to have "w=" "h=" "regions=" markers
        let line0 = get_line(&buf, 0);
        let line1 = get_line(&buf, 1);
        let line2 = get_line(&buf, 2);

        assert!(
            !line0.contains("w=") && !line0.contains("regions="),
            "line 0 should not have debug output: '{}'",
            line0.trim()
        );
        assert!(
            !line1.contains("room=") && !line1.contains("history="),
            "line 1 should not have debug output: '{}'",
            line1.trim()
        );
        assert!(
            !line2.contains("CHAT:"),
            "line 2 should not have CHAT debug output: '{}'",
            line2.trim()
        );

        // Status should be at y=22
        let status = get_line(&buf, 22);
        assert!(
            status.contains("[") || status.contains("─"),
            "status line (y=22) should have status content"
        );

        // Input should be at y=23 (may be empty prompt or have content)
        let input = get_line(&buf, 23);
        // Just verify it doesn't have debug output
        assert!(
            !input.contains("CHAT:") && !input.contains("regions="),
            "input line (y=23) should not have debug output: '{}'",
            input.trim()
        );
    }

    #[test]
    fn test_layout_tiny_terminal_1x1() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Test that layout doesn't panic or produce negative values for 1x1 terminal
        let result: bool = runtime
            .lua
            .load(
                r#"
                local layout = require 'ui.layout'

                -- Create tiny 1x1 rect
                local r = layout.rect(0, 0, 1, 1)
                assert(r.x == 0, "x should be 0")
                assert(r.y == 0, "y should be 0")
                assert(r.w == 1, "w should be 1")
                assert(r.h == 1, "h should be 1")

                -- Shrink should clamp to 0, not go negative
                local shrunk = layout.shrink(r, 1, 1, 1, 1)
                assert(shrunk.w >= 0, "shrunk width should not be negative")
                assert(shrunk.h >= 0, "shrunk height should not be negative")

                -- Sub rect should also clamp
                local sub = layout.sub(r, 0, 0, 10, 10)
                assert(sub.w <= r.w, "sub width should be clamped to parent")
                assert(sub.h <= r.h, "sub height should be clamped to parent")

                return true
            "#,
            )
            .eval()
            .expect("tiny terminal test should not panic");

        assert!(result, "1x1 terminal layout should handle gracefully");
    }

    #[test]
    fn test_layout_tiny_terminal_2x2() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Test 2x2 terminal with bar system
        let result: bool = runtime
            .lua
            .load(
                r#"
                local layout = require 'ui.layout'
                local bars = require 'ui.bars'

                -- Clear any existing bars
                bars.clear()

                -- Define minimal bars
                bars.define("status", {
                    position = "bottom",
                    priority = 50,
                    height = 1,
                    items = {},
                })
                bars.define("input", {
                    position = "bottom",
                    priority = 100,
                    height = 1,
                    items = {},
                })

                -- Compute layout for 2x2
                local result = bars.compute_layout(2, 2, {})

                -- Both bars want bottom space, total 2 lines
                -- On 2x2 terminal, content gets 0 height (which is valid)
                assert(result.content ~= nil, "content area should exist")
                assert(result.content.height >= 0, "content height should not be negative")

                -- Status and input should be present
                assert(result.status ~= nil or result.input ~= nil, "at least one bar should fit")

                return true
            "#,
            )
            .eval()
            .expect("2x2 terminal test should not panic");

        assert!(result, "2x2 terminal layout should handle gracefully");
    }

    #[test]
    fn test_bar_priority_stacking() {
        let runtime = LuaRuntime::new().expect("should create runtime");

        // Test that bars with higher priority stack closer to the edge
        let result: bool = runtime
            .lua
            .load(
                r#"
                local bars = require 'ui.bars'

                -- Clear any existing bars
                bars.clear()

                -- Define bars with different priorities
                -- Higher priority = closer to edge
                bars.define("input", {
                    position = "bottom",
                    priority = 100,  -- highest, should be at very bottom
                    height = 1,
                    items = {},
                })
                bars.define("status", {
                    position = "bottom",
                    priority = 50,  -- lower, should be above input
                    height = 1,
                    items = {},
                })

                -- Compute layout for 80x24
                local result = bars.compute_layout(80, 24, {})

                -- Layout is 1-indexed (rows 1-24)
                -- Input (priority 100) should be at row 24 (last row)
                assert(result.input ~= nil, "input bar should exist")
                assert(result.input.row == 24, "input (priority 100) should be at row 24, got " .. tostring(result.input.row))

                -- Status (priority 50) should be at row 23 (above input)
                assert(result.status ~= nil, "status bar should exist")
                assert(result.status.row == 23, "status (priority 50) should be at row 23, got " .. tostring(result.status.row))

                -- Content should get remaining space (rows 1-22)
                assert(result.content ~= nil, "content should exist")
                assert(result.content.row == 1, "content should start at row 1")
                assert(result.content.height == 22, "content should have height 22, got " .. tostring(result.content.height))

                return true
            "#,
            )
            .eval()
            .expect("bar priority test should run");

        assert!(result, "bars should stack by priority (higher = closer to edge)");
    }
}
