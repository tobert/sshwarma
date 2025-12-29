//! Lua integration for sshwarma
//!
//! This module provides Lua scripting support for customizable HUD rendering.
//! Supports hot-reloading of user scripts from ~/.config/sshwarma/hud.lua.

pub mod cache;
pub mod context;
pub mod mcp_bridge;
pub mod render;
pub mod tools;

pub use cache::ToolCache;
pub use context::{build_hud_context, PendingNotification};
pub use mcp_bridge::{mcp_request_handler, McpBridge};
pub use render::{parse_lua_output, HUD_ROWS};
pub use tools::{register_mcp_tools, LuaToolState};

use crate::display::hud::HudState;
use crate::lua::tools::register_tools;
use anyhow::{Context, Result};
use mlua::{Lua, Table, Value};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use tracing::{debug, info, warn};

/// Embedded default HUD script
const DEFAULT_HUD_SCRIPT: &str = include_str!("../embedded/hud.lua");

/// Get the XDG config path for sshwarma
fn config_path() -> Option<PathBuf> {
    // Try XDG_CONFIG_HOME first
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("sshwarma"));
    }

    // Fall back to ~/.config
    if let Ok(home) = std::env::var("HOME") {
        return Some(PathBuf::from(home).join(".config/sshwarma"));
    }

    None
}

/// User config directory for custom scripts
fn user_config_dir() -> PathBuf {
    config_path().unwrap_or_else(|| PathBuf::from(".config/sshwarma"))
}

/// Path to user's custom HUD script
pub fn user_hud_script_path() -> PathBuf {
    user_config_dir().join("hud.lua")
}

/// Path to a specific user's HUD script (e.g., atobey.lua, claude.lua)
pub fn user_named_script_path(username: &str) -> PathBuf {
    user_config_dir().join(format!("{}.lua", username))
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

        // Register tool functions
        register_tools(&lua, tool_state.clone())
            .map_err(|e| anyhow::anyhow!("failed to register Lua tools: {}", e))?;

        // Load the default embedded script
        lua.load(DEFAULT_HUD_SCRIPT)
            .set_name("embedded:hud.lua")
            .exec()
            .map_err(|e| anyhow::anyhow!("failed to load embedded HUD script: {}", e))?;

        info!("Lua runtime initialized with embedded HUD script");

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

    /// Load a Lua script from file
    ///
    /// Replaces the current script. The script must define a `render_hud` function.
    pub fn load_script(&mut self, path: &PathBuf) -> Result<()> {
        let script =
            fs::read_to_string(path).with_context(|| format!("failed to read script {:?}", path))?;

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
}
