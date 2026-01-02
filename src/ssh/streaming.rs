//! Streaming response handling
//!
//! Handles model response streaming with Row updates.
//! Updates are written to the database; Lua's on_tick renders them via tools.history().

use crate::db::Database;
use crate::lua::LuaRuntime;
use crate::status::Status;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Update from background task for streaming responses
#[derive(Debug)]
pub enum RowUpdate {
    /// Incremental text chunk (for streaming)
    Chunk { row_id: String, text: String },
    /// Tool being invoked
    ToolCall {
        row_id: String,
        tool_name: String,
        model_name: String,
    },
    /// Tool result received
    ToolResult { row_id: String, summary: String },
    /// Stream completed
    Complete { row_id: String, model_name: String },
}

/// Background task that processes streaming updates
///
/// Updates are written to the database only - Lua's on_tick renders them
/// via tools.history() at ~100ms intervals.
pub async fn push_updates_task(
    mut rx: mpsc::Receiver<RowUpdate>,
    db: Arc<Database>,
    lua_runtime: Option<Arc<Mutex<LuaRuntime>>>,
) {
    while let Some(update) = rx.recv().await {
        match update {
            RowUpdate::Chunk { row_id, text } => {
                // Append to database row - Lua will render via tools.history()
                if let Err(e) = db.append_to_row(&row_id, &text) {
                    tracing::error!("failed to append to row: {}", e);
                }
            }

            RowUpdate::ToolCall {
                row_id,
                tool_name,
                model_name,
            } => {
                // Update status for Lua HUD
                if let Some(ref lua_runtime) = lua_runtime {
                    let lua = lua_runtime.lock().await;
                    lua.tool_state()
                        .set_status(&model_name, Status::RunningTool(tool_name.clone()));
                }

                // Update row to show tool call
                if let Ok(Some(mut row)) = db.get_row(&row_id) {
                    row.content = Some(format!(
                        "{}[calling {}...]",
                        row.content.unwrap_or_default(),
                        tool_name
                    ));
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

                // Update status to idle for Lua HUD
                if let Some(ref lua_runtime) = lua_runtime {
                    let lua = lua_runtime.lock().await;
                    lua.tool_state().set_status(&model_name, Status::Idle);
                }
            }
        }
    }
}
