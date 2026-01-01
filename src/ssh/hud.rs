//! HUD refresh task
//!
//! Periodically refreshes the heads-up display at the bottom of the terminal.
//!
//! Lua defines on_tick(tick, ctx) and draws directly to a buffer.
//! Output is only sent when the buffer content changes.
//!
//! Also executes room rules on tick/interval triggers.

use crate::db::rules::ActionSlot;
use crate::lua::LuaRuntime;
use crate::state::SharedState;
use crate::terminal::HUD_HEIGHT;
use crate::ui::RenderBuffer;
use russh::server::Handle;
use russh::{ChannelId, CryptoVec};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;

/// Spawn the HUD refresh task
pub fn spawn_hud_refresh(
    handle: Handle,
    channel: ChannelId,
    lua_runtime: Arc<TokioMutex<LuaRuntime>>,
    state: Arc<SharedState>,
    term_width: u16,
    term_height: u16,
) {
    tokio::spawn(async move {
        hud_refresh_task(
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

/// HUD refresh loop (100ms interval)
async fn hud_refresh_task(
    handle: Handle,
    channel: ChannelId,
    lua_runtime: Arc<TokioMutex<LuaRuntime>>,
    state: Arc<SharedState>,
    term_width: u16,
    term_height: u16,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    let mut tick: u64 = 0;

    // Create render buffer for HUD region (full width, HUD_HEIGHT lines)
    let render_buffer = Arc::new(Mutex::new(RenderBuffer::new(term_width, HUD_HEIGHT)));
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
            // Get room name from session context
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

        // Render via on_tick
        let output = {
            let lua = lua_runtime.lock().await;

            // Clear buffer before drawing
            {
                let mut buf = render_buffer.lock().unwrap();
                buf.clear();
            }

            // Call on_tick - Lua draws to buffer
            // Lua queries live data via sshwarma.call("status")
            if let Err(e) = lua.call_on_tick(tick, render_buffer.clone(), term_width, HUD_HEIGHT) {
                tracing::debug!("on_tick error: {}", e);
                continue;
            }

            // Get ANSI output from buffer with absolute positioning
            // to_ansi_at avoids newlines that would cause scrolling
            let buf = render_buffer.lock().unwrap();
            let hud_row = term_height.saturating_sub(HUD_HEIGHT);
            buf.to_ansi_at(hud_row)
        };

        // Only send if changed
        if output != last_output {
            last_output = output.clone();
            if handle.data(channel, CryptoVec::from(output.as_bytes())).await.is_err() {
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
