//! Streaming response handling
//!
//! Handles model response streaming with Row updates.

use crate::db::Database;
use crate::display::hud::{HudState, ParticipantStatus, HUD_HEIGHT};
use crate::display::styles::ctrl;
use crate::ui::render::render_rows;
use russh::server::Handle;
use russh::{ChannelId, CryptoVec};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Update from background task for streaming responses
#[derive(Debug)]
pub enum RowUpdate {
    /// Incremental text chunk (for streaming)
    Chunk {
        row_id: String,
        text: String,
    },
    /// Tool being invoked
    ToolCall {
        row_id: String,
        tool_name: String,
    },
    /// Tool result received
    ToolResult {
        row_id: String,
        summary: String,
    },
    /// Stream completed
    Complete {
        row_id: String,
        model_name: String,
    },
}

/// Background task that processes streaming updates
pub async fn push_updates_task(
    handle: Handle,
    channel: ChannelId,
    mut rx: mpsc::Receiver<RowUpdate>,
    db: Arc<Database>,
    buffer_id: String,
    hud_state: Arc<Mutex<HudState>>,
    term_width: u16,
    term_height: u16,
) {
    let mut _accumulated_text = String::new();
    let mut _current_row_id: Option<String> = None;

    while let Some(update) = rx.recv().await {
        match update {
            RowUpdate::Chunk { row_id, text } => {
                // Append to database row
                if let Err(e) = db.append_to_row(&row_id, &text) {
                    tracing::error!("failed to append to row: {}", e);
                    continue;
                }

                _accumulated_text.push_str(&text);
                _current_row_id = Some(row_id.clone());

                // HUD status updates happen through the HUD refresh task

                // Render updated row
                if let Ok(Some(row)) = db.get_row(&row_id) {
                    let rendered = render_rows(&[row], term_width as usize);
                    let output = format_streaming_output(&rendered, term_width, term_height);
                    let _ = handle
                        .data(channel, CryptoVec::from(output.as_bytes()))
                        .await;
                }
            }

            RowUpdate::ToolCall { row_id, tool_name } => {
                // Update HUD to show tool running
                {
                    let mut hud = hud_state.lock().await;
                    if let Some(participant) = hud.participants.iter_mut().find(|p| p.kind == crate::display::hud::ParticipantKind::Model) {
                        participant.status = ParticipantStatus::RunningTool(tool_name.clone());
                    }
                }

                // Update row to show tool call
                if let Ok(Some(mut row)) = db.get_row(&row_id) {
                    row.content = Some(format!("{}[calling {}...]",
                        row.content.unwrap_or_default(), tool_name));
                    let _ = db.update_row(&row);
                }
            }

            RowUpdate::ToolResult { row_id, summary } => {
                // Append tool result to row
                if let Err(e) = db.append_to_row(&row_id, &format!("\n{}", summary)) {
                    tracing::error!("failed to append tool result: {}", e);
                }
            }

            RowUpdate::Complete { row_id, model_name } => {
                // Finalize the row
                if let Err(e) = db.finalize_row(&row_id) {
                    tracing::error!("failed to finalize row: {}", e);
                }

                // Update HUD to idle
                {
                    let mut hud = hud_state.lock().await;
                    hud.update_status(&model_name, ParticipantStatus::Idle);
                }

                // Full re-render to clean up
                if let Ok(rows) = db.list_buffer_rows(&buffer_id) {
                    let rendered = render_rows(&rows, term_width as usize);
                    let output = format_full_render(&rendered, term_width, term_height);
                    let _ = handle
                        .data(channel, CryptoVec::from(output.as_bytes()))
                        .await;
                }

                // HUD is redrawn by the refresh task - no need to redraw here

                _accumulated_text.clear();
                _current_row_id = None;
            }
        }
    }
}

/// Format output for streaming update (partial line replacement)
fn format_streaming_output(rendered: &str, _width: u16, _height: u16) -> String {
    // Move to start of line, clear, write new content
    format!("{}{}{}", ctrl::CR, ctrl::clear_line(), rendered)
}

/// Format output for full re-render
fn format_full_render(rendered: &str, _width: u16, height: u16) -> String {
    let mut output = String::new();
    // Move to top, clear scroll region, redraw
    output.push_str(&ctrl::move_to(1, 1));
    for _ in 0..height.saturating_sub(HUD_HEIGHT + 1) {
        output.push_str(&ctrl::clear_line());
        output.push_str(ctrl::CRLF);
    }
    output.push_str(&ctrl::move_to(1, 1));
    output.push_str(rendered);
    output
}
