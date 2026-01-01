//! HUD refresh task
//!
//! Periodically refreshes the heads-up display at the bottom of the terminal.

use crate::display::hud::{HudState, HUD_HEIGHT};
use crate::display::styles::ctrl;
use crate::lua::LuaRuntime;
use crate::state::SharedState;
use chrono::Utc;
use russh::server::Handle;
use russh::{ChannelId, CryptoVec};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Spawn the HUD refresh task
pub fn spawn_hud_refresh(
    handle: Handle,
    channel: ChannelId,
    hud_state: Arc<Mutex<HudState>>,
    lua_runtime: Arc<Mutex<LuaRuntime>>,
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
    hud_state: Arc<Mutex<HudState>>,
    lua_runtime: Arc<Mutex<LuaRuntime>>,
    state: Arc<SharedState>,
    term_width: u16,
    term_height: u16,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    let mut tick: u64 = 0;

    loop {
        interval.tick().await;
        tick += 1;

        // Update MCP connections every second
        if tick.is_multiple_of(10) {
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
        if tick.is_multiple_of(5) {
            let lua = lua_runtime.lock().await;
            let background_tick = tick / 5;
            if let Err(e) = lua.call_background(background_tick) {
                tracing::debug!("lua background error: {}", e);
            }
        }

        // Render HUD
        let output = {
            let hud = hud_state.lock().await;
            let lua = lua_runtime.lock().await;
            lua.update_state(hud.clone());

            let now_ms = Utc::now().timestamp_millis();
            match lua.render_hud_string(now_ms, term_width, term_height) {
                Ok(rendered) => {
                    let hud_row = term_height.saturating_sub(HUD_HEIGHT);
                    format!("{}{}", ctrl::move_to(hud_row, 1), rendered)
                }
                Err(e) => {
                    tracing::debug!("hud render error: {}", e);
                    continue;
                }
            }
        };

        // Send to client
        if handle.data(channel, CryptoVec::from(output.as_bytes())).await.is_err() {
            // Connection closed
            break;
        }
    }
}
