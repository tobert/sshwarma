//! Context composition for LLM interactions
//!
//! The `wrap()` system composes context from multiple sources into:
//! 1. **System prompt** - Stable identity, passed via rig's `.preamble()` (enables caching)
//! 2. **Context** - Dynamic room state, prepended to user message
//!
//! Lua scripts control what gets included via a lazy builder pattern.

use crate::model::ModelHandle;
use crate::state::SharedState;
use anyhow::Result;
use mlua::{Function, Lua, Table};
use std::sync::Arc;

/// State needed for wrap operations
///
/// Passed to Lua via registry so wrap tools can access sshwarma state.
#[derive(Clone)]
pub struct WrapState {
    /// Current room name (None if in lobby)
    pub room_name: Option<String>,
    /// Username of the person who triggered the @mention
    pub username: String,
    /// Model being addressed
    pub model: ModelHandle,
    /// Shared application state for DB, world, etc.
    pub shared_state: Arc<SharedState>,
}

/// Result of context composition
#[derive(Debug)]
pub struct WrapResult {
    /// Stable system prompt (for .preamble())
    pub system_prompt: String,
    /// Dynamic context (prepended to user message)
    pub context: String,
}

/// Compose context using Lua wrap() function
///
/// Calls the Lua `default_wrap(target_tokens)` function which returns
/// a WrapBuilder, then calls `:system_prompt()` and `:context()` on it.
///
/// # Arguments
/// * `lua` - The Lua runtime
/// * `target_tokens` - Token budget for context composition
///
/// # Returns
/// WrapResult with system_prompt and context strings
pub fn compose_context(lua: &Lua, target_tokens: usize) -> Result<WrapResult> {
    let globals = lua.globals();

    // Call default_wrap(target_tokens) to get the builder
    let default_wrap_fn: Function = globals
        .get("default_wrap")
        .map_err(|e| anyhow::anyhow!("default_wrap function not found: {}", e))?;

    let builder: Table = default_wrap_fn
        .call(target_tokens as i64)
        .map_err(|e| anyhow::anyhow!("default_wrap() call failed: {}", e))?;

    // Call builder:system_prompt()
    let system_prompt_fn: Function = builder
        .get("system_prompt")
        .map_err(|e| anyhow::anyhow!("system_prompt method not found: {}", e))?;

    let system_prompt: String = system_prompt_fn
        .call(builder.clone())
        .map_err(|e| anyhow::anyhow!("system_prompt() call failed: {}", e))?;

    // Call builder:context()
    let context_fn: Function = builder
        .get("context")
        .map_err(|e| anyhow::anyhow!("context method not found: {}", e))?;

    let context: String = context_fn
        .call(builder)
        .map_err(|e| anyhow::anyhow!("context() call failed: {}", e))?;

    Ok(WrapResult {
        system_prompt,
        context,
    })
}

