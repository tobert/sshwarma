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

/// Event-driven screen refresh
///
/// Waits for dirty signal instead of polling. Background tasks run on 5-second intervals.
async fn screen_refresh_task(
    handle: Handle,
    channel: ChannelId,
    lua_runtime: Arc<TokioMutex<LuaRuntime>>,
    state: Arc<SharedState>,
    term_width: u16,
    term_height: u16,
) {
    // Get the dirty signal for event-driven updates
    let dirty_signal = {
        let lua = lua_runtime.lock().await;
        lua.tool_state().dirty_signal()
    };

    // Create full-screen render buffer - Lua owns everything including input
    let render_buffer = Arc::new(Mutex::new(RenderBuffer::new(term_width, term_height)));
    let mut last_output = String::new();
    let mut tick: u64 = 0;
    let mut background_tick: u64 = 0;

    // Initial render
    render_screen(
        &handle,
        channel,
        &lua_runtime,
        &render_buffer,
        &mut last_output,
        tick,
        term_width,
        term_height,
    )
    .await;

    loop {
        // Wait for either:
        // 1. Dirty signal (something changed, redraw immediately)
        // 2. 5-second timeout (run background tasks)
        let was_dirty = tokio::select! {
            _ = dirty_signal.notified() => true,
            _ = tokio::time::sleep(Duration::from_secs(5)) => false,
        };

        tick += 1;

        if was_dirty {
            // Something changed - render immediately
            if !render_screen(
                &handle,
                channel,
                &lua_runtime,
                &render_buffer,
                &mut last_output,
                tick,
                term_width,
                term_height,
            )
            .await
            {
                break; // Connection closed
            }
        } else {
            // 5-second background tick
            background_tick += 1;

            // Run user's background() function
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

            // Render after background tasks (they might have changed state)
            if !render_screen(
                &handle,
                channel,
                &lua_runtime,
                &render_buffer,
                &mut last_output,
                tick,
                term_width,
                term_height,
            )
            .await
            {
                break;
            }
        }
    }
}

/// Render the screen via Lua's on_tick. Returns false if connection closed.
async fn render_screen(
    handle: &Handle,
    channel: ChannelId,
    lua_runtime: &Arc<TokioMutex<LuaRuntime>>,
    render_buffer: &Arc<Mutex<RenderBuffer>>,
    last_output: &mut String,
    tick: u64,
    term_width: u16,
    term_height: u16,
) -> bool {
    let output = {
        let lua = lua_runtime.lock().await;

        // Clear buffer before drawing
        {
            let mut buf = render_buffer.lock().unwrap();
            buf.clear();
        }

        // Call on_tick - Lua draws to full screen including input
        if let Err(e) = lua.call_on_tick(tick, render_buffer.clone(), term_width, term_height) {
            tracing::debug!("on_tick error: {}", e);
            return true; // Continue, just skip this frame
        }

        // Get ANSI output from buffer, positioned at top-left
        let buf = render_buffer.lock().unwrap();
        buf.to_ansi_at(1)
    };

    // Only send if changed
    if output != *last_output {
        *last_output = output.clone();

        // Wrap in synchronized output to prevent tearing
        let final_output = format!(
            "\x1b[?2026h{}\x1b[{};1H\x1b[?25h\x1b[?2026l",
            output, term_height
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
