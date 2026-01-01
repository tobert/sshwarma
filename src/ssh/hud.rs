//! HUD refresh task
//!
//! Periodically refreshes the heads-up display at the bottom of the terminal.
//!
//! Lua defines on_tick(tick, ctx) and draws directly to a buffer.
//! Output is only sent when the buffer content changes.
//!
//! Also executes room rules on tick/interval triggers.

use crate::db::rules::ActionSlot;
use crate::display::hud::{HudState, HUD_HEIGHT};
use crate::lua::LuaRuntime;
use crate::state::SharedState;
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
    hud_state: Arc<TokioMutex<HudState>>,
    lua_runtime: Arc<TokioMutex<LuaRuntime>>,
    state: Arc<SharedState>,
    term_width: u16,
    term_height: u16,
) {
    tokio::spawn(async move {
        hud_refresh_task(
            handle,
            channel,
            hud_state,
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
    hud_state: Arc<TokioMutex<HudState>>,
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

        // Update MCP connections every second
        if tick % 10 == 0 {
            let connections = state.mcp.list_connections().await;
            let mut hud = hud_state.lock().await;
            hud.set_mcp_connections(
                connections
                    .into_iter()
                    .map(|c| crate::display::hud::McpConnectionState {
                        name: c.name,
                        tool_count: c.tool_count,
                        connected: true,
                        call_count: c.call_count,
                        last_tool: c.last_tool,
                    })
                    .collect(),
            );
        }

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
                let hud = hud_state.lock().await;
                hud.room_name.clone()
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
            let hud = hud_state.lock().await;
            let lua = lua_runtime.lock().await;
            lua.update_state(hud.clone());

            // Clear buffer before drawing
            {
                let mut buf = render_buffer.lock().unwrap();
                buf.clear();
            }

            // Call on_tick - Lua draws to buffer
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
