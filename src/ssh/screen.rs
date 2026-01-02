//! Screen refresh task
//!
//! Periodically calls Lua's on_tick() with a full-screen buffer.
//! Lua owns the entire screen layout - chat, status, input, everything.
//!
//! Also executes room rules on tick/interval triggers.

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

/// Screen refresh loop (100ms interval)
async fn screen_refresh_task(
    handle: Handle,
    channel: ChannelId,
    lua_runtime: Arc<TokioMutex<LuaRuntime>>,
    state: Arc<SharedState>,
    term_width: u16,
    term_height: u16,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    let mut tick: u64 = 0;

    // Create full-screen render buffer - Lua owns everything including input
    let render_buffer = Arc::new(Mutex::new(RenderBuffer::new(term_width, term_height)));
    let mut last_output = String::new();

    loop {
        interval.tick().await;
        tick += 1;

        // Run Lua background function every 500ms (tick % 5)
        if tick % 5 == 0 {
            let background_tick = tick / 5;

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
                // Advance rules engine tick counter
                state.rules.tick();

                // Execute tick-triggered rules
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

                // Execute interval-triggered rules
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

        // Render via on_tick - Lua gets full screen
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
                continue;
            }

            // Get ANSI output from buffer, positioned at top-left
            let buf = render_buffer.lock().unwrap();
            buf.to_ansi_at(1) // Row 1 (1-indexed for terminal)
        };

        // Only send if changed
        if output != last_output {
            last_output = output.clone();
            if handle
                .data(channel, CryptoVec::from(output.as_bytes()))
                .await
                .is_err()
            {
                // Connection closed
                break;
            }
        }
    }
}

/// Execute a rule script by loading it from the database
async fn execute_rule_script(
    lua_runtime: &Arc<TokioMutex<LuaRuntime>>,
    state: &Arc<SharedState>,
    script_id: &str,
    tick: u64,
) {
    // Load script from database
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

    // Execute the script
    let lua = lua_runtime.lock().await;
    let script_name = script.name.as_deref().unwrap_or(script_id);
    if let Err(e) = lua.execute_rule_script(&script.code, script_name, tick) {
        tracing::debug!("rule script '{}' error: {}", script_name, e);
    }
}
