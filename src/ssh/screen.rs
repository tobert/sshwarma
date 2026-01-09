//! Screen refresh task
//!
//! Event-driven rendering - only redraws when state changes.
//! Lua owns the entire screen layout - chat, status, input, everything.

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
        screen_refresh_task(handle, channel, lua_runtime, state, term_width, term_height).await;
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
    _state: Arc<SharedState>,
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
    tracing::info!("screen_refresh_task: marked initial dirty tags");

    // Do initial render immediately (don't wait for notify - it was already sent)
    let dirty_tags = dirty.take();
    tracing::info!("screen_refresh_task: initial dirty_tags = {:?}", dirty_tags);
    if !dirty_tags.is_empty() {
        tracing::info!("screen_refresh_task: doing initial render");
        if !render_screen_with_tags(
            &handle,
            channel,
            &lua_runtime,
            &current_buffer,
            &mut last_buffer,
            &dirty_tags,
            0,
            term_width,
            term_height,
        )
        .await
        {
            return; // Connection closed
        }
    }

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
#[allow(clippy::too_many_arguments)]
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
        tracing::debug!("render_screen_with_tags: calling on_tick");
        if let Err(e) = lua.call_on_tick_with_tags(
            dirty_tags,
            tick,
            current_buffer.clone(),
            term_width,
            term_height,
        ) {
            tracing::warn!("on_tick error: {}", e);
            return true; // Continue, just skip this frame
        }
        tracing::debug!("render_screen_with_tags: on_tick completed");

        // Generate diff ANSI - only rows that changed
        let buf = current_buffer.lock().unwrap();
        buf.diff_ansi(last_buffer, 0)
    };

    // Update last_buffer for next comparison
    {
        let buf = current_buffer.lock().unwrap();
        *last_buffer = buf.clone();
    }

    // Get cursor position reported by Lua (set via tools.set_cursor_pos)
    let (cursor_row, cursor_col) = {
        let lua = lua_runtime.lock().await;
        let input = lua.tool_state().input_state();
        (input.cursor_row, input.cursor_col)
    };

    // Only send if there are changes
    tracing::debug!(
        "render_screen_with_tags: diff_output len = {}",
        diff_output.len()
    );
    if !diff_output.is_empty() {
        // Wrap in synchronized output to prevent tearing.
        // Position hardware cursor at Lua-reported position for layered blink effect.
        // Lua renders a visual cursor (styled background), hardware cursor overlays it.
        let final_output = if cursor_row > 0 && cursor_col > 0 {
            format!(
                "\x1b[?2026h{}\x1b[{};{}H\x1b[?25h\x1b[?2026l",
                diff_output, cursor_row, cursor_col
            )
        } else {
            // No cursor position reported yet - hide hardware cursor
            format!("\x1b[?2026h{}\x1b[?25l\x1b[?2026l", diff_output)
        };

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
