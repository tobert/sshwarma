//! Screen refresh task
//!
//! Event-driven rendering - only redraws when state changes.
//! Lua owns the entire screen layout - chat, status, input, everything.
//!
//! Also executes room rules on background tick triggers.

use crate::db::rules::ActionSlot;
use crate::lua::LuaRuntime;
use crate::state::SharedState;
use crate::ui::RenderBuffer;
use russh::server::Handle;
use russh::{ChannelId, CryptoVec};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;

/// Spawn the screen refresh task
pub fn spawn_screen_refresh(
    handle: Handle,
    channel: ChannelId,
    lua_runtime: Arc<TokioMutex<LuaRuntime>>,
    state: Arc<SharedState>,
    term_width: u16,
    term_height: u16,
) {
    tokio::spawn(async move {
        screen_refresh_task(
            handle,
            channel,
            lua_runtime,
            state,
            term_width,
            term_height,
        )
        .await;
    });
}

/// Event-driven screen refresh with tag-based dirty tracking
///
/// Uses double-buffering and row-based diffing for efficient partial updates.
/// Only redraws rows that actually changed, preserving terminal selection.
async fn screen_refresh_task(
    handle: Handle,
    channel: ChannelId,
    lua_runtime: Arc<TokioMutex<LuaRuntime>>,
    state: Arc<SharedState>,
    term_width: u16,
    term_height: u16,
) {
    // Get the dirty state for event-driven updates
    let dirty = {
        let lua = lua_runtime.lock().await;
        lua.tool_state().dirty().clone()
    };

    // Double-buffered rendering for efficient diffing
    let current_buffer = Arc::new(Mutex::new(RenderBuffer::new(term_width, term_height)));
    let mut last_buffer = RenderBuffer::new(term_width, term_height);
    let mut tick: u64 = 0;
    let mut background_tick: u64 = 0;

    // Mark all regions dirty for initial render
    dirty.mark_many(["status", "chat", "input"]);

    loop {
        // Wait for either:
        // 1. Dirty signal (something changed, redraw)
        // 2. 500ms timeout (run background tasks)
        let was_background = tokio::select! {
            _ = dirty.notified() => false,
            _ = tokio::time::sleep(Duration::from_millis(500)) => true,
        };

        tick += 1;

        if was_background {
            // 500ms background tick
            background_tick += 1;

            // Run user's background() function
            // (This can call tools.mark_dirty() to trigger redraws)
            {
                let lua = lua_runtime.lock().await;
                if let Err(e) = lua.call_background(background_tick) {
                    tracing::debug!("lua background error: {}", e);
                }
            }

            // Execute room rules (tick and interval triggers)
            let room_name = {
                let lua = lua_runtime.lock().await;
                lua.tool_state()
                    .session_context()
                    .and_then(|ctx| ctx.room_name.clone())
            };

            if let Some(ref room_id) = room_name {
                state.rules.tick();

                if let Ok(matches) = state.rules.match_tick(&state.db, room_id) {
                    for rule_match in matches {
                        if rule_match.rule.action_slot == ActionSlot::Background {
                            execute_rule_script(
                                &lua_runtime,
                                &state,
                                &rule_match.rule.script_id,
                                background_tick,
                            )
                            .await;
                        }
                    }
                }

                if let Ok(matches) = state.rules.match_interval(&state.db, room_id) {
                    for rule_match in matches {
                        if rule_match.rule.action_slot == ActionSlot::Background {
                            execute_rule_script(
                                &lua_runtime,
                                &state,
                                &rule_match.rule.script_id,
                                background_tick,
                            )
                            .await;
                        }
                    }
                }
            }
        }

        // Get dirty tags (clears the set)
        let dirty_tags = dirty.take();
        if dirty_tags.is_empty() {
            continue; // Nothing to redraw
        }

        // Render screen with dirty tags
        if !render_screen_with_tags(
            &handle,
            channel,
            &lua_runtime,
            &current_buffer,
            &mut last_buffer,
            &dirty_tags,
            tick,
            term_width,
            term_height,
        )
        .await
        {
            break; // Connection closed
        }
    }
}

/// Render the screen with tag-based dirty tracking and row diffing.
/// Returns false if connection closed.
async fn render_screen_with_tags(
    handle: &Handle,
    channel: ChannelId,
    lua_runtime: &Arc<TokioMutex<LuaRuntime>>,
    current_buffer: &Arc<Mutex<RenderBuffer>>,
    last_buffer: &mut RenderBuffer,
    dirty_tags: &std::collections::HashSet<String>,
    tick: u64,
    term_width: u16,
    term_height: u16,
) -> bool {
    let diff_output = {
        let lua = lua_runtime.lock().await;

        // Clear current buffer before drawing
        {
            let mut buf = current_buffer.lock().unwrap();
            buf.clear();
        }

        // Call on_tick with dirty tags - Lua draws to full screen
        // Future: Lua can use dirty_tags to render only affected regions
        if let Err(e) =
            lua.call_on_tick_with_tags(dirty_tags, tick, current_buffer.clone(), term_width, term_height)
        {
            tracing::debug!("on_tick error: {}", e);
            return true; // Continue, just skip this frame
        }

        // Generate diff ANSI - only rows that changed
        let buf = current_buffer.lock().unwrap();
        buf.diff_ansi(last_buffer, 0)
    };

    // Update last_buffer for next comparison
    {
        let buf = current_buffer.lock().unwrap();
        *last_buffer = buf.clone();
    }

    // Only send if there are changes
    if !diff_output.is_empty() {
        // Wrap in synchronized output to prevent tearing
        let final_output = format!(
            "\x1b[?2026h{}\x1b[{};1H\x1b[?25h\x1b[?2026l",
            diff_output, term_height
        );

        if handle
            .data(channel, CryptoVec::from(final_output.as_bytes()))
            .await
            .is_err()
        {
            return false; // Connection closed
        }
    }

    true
}

/// Execute a rule script by loading it from the database
async fn execute_rule_script(
    lua_runtime: &Arc<TokioMutex<LuaRuntime>>,
    state: &Arc<SharedState>,
    script_id: &str,
    tick: u64,
) {
    let script = match state.db.get_script(script_id) {
        Ok(Some(s)) => s,
        Ok(None) => {
            tracing::warn!("rule script not found: {}", script_id);
            return;
        }
        Err(e) => {
            tracing::warn!("failed to load rule script {}: {}", script_id, e);
            return;
        }
    };

    let lua = lua_runtime.lock().await;
    let script_name = script.name.as_deref().unwrap_or(script_id);
    if let Err(e) = lua.execute_rule_script(&script.code, script_name, tick) {
        tracing::debug!("rule script '{}' error: {}", script_name, e);
    }
}
