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
        tool_args: Option<String>,
        model_name: String,
        buffer_id: String,
        agent_id: String,
    },
    /// Tool result received
    ToolResult {
        row_id: String,
        tool_name: String,
        summary: String,
        success: bool,
        buffer_id: String,
    },
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
                // Signal chat region needs refresh
                if let Some(ref lua_runtime) = lua_runtime {
                    let lua = lua_runtime.lock().await;
                    lua.tool_state().mark_dirty("chat");
                }
            }

            RowUpdate::ToolCall {
                row_id,
                tool_name,
                tool_args,
                model_name,
                buffer_id,
                agent_id,
            } => {
                // Update status for Lua HUD
                if let Some(ref lua_runtime) = lua_runtime {
                    let lua = lua_runtime.lock().await;
                    lua.tool_state()
                        .set_status(&model_name, Status::RunningTool(tool_name.clone()));
                }

                // Create a proper tool.call row linked to the model message
                let mut tool_row = crate::db::rows::Row::tool_call_with_parent(
                    &buffer_id,
                    &row_id,
                    &agent_id,
                    &tool_name,
                    tool_args.as_ref(),
                );
                if let Err(e) = db.append_row(&mut tool_row) {
                    tracing::error!("failed to create tool call row: {}", e);
                }

                // Signal chat region needs refresh
                if let Some(ref lua_runtime) = lua_runtime {
                    let lua = lua_runtime.lock().await;
                    lua.tool_state().mark_dirty("chat");
                }
            }

            RowUpdate::ToolResult {
                row_id,
                tool_name,
                summary,
                success,
                buffer_id,
            } => {
                // Create a proper tool.result row linked to the model message
                let mut result_row = crate::db::rows::Row::tool_result_with_parent(
                    &buffer_id,
                    &row_id,
                    &tool_name,
                    &summary,
                    success,
                );
                if let Err(e) = db.append_row(&mut result_row) {
                    tracing::error!("failed to create tool result row: {}", e);
                }

                // Signal chat region needs refresh
                if let Some(ref lua_runtime) = lua_runtime {
                    let lua = lua_runtime.lock().await;
                    lua.tool_state().mark_dirty("chat");
                }
            }

            RowUpdate::Complete { row_id, model_name } => {
                // Get the thinking.stream row content to create final message
                if let Ok(Some(thinking_row)) = db.get_row(&row_id) {
                    if let Some(content) = thinking_row.content {
                        // Create a new message.model row with the final content
                        let mut message_row = crate::db::rows::Row::new(
                            &thinking_row.buffer_id,
                            "message.model",
                        );
                        message_row.source_agent_id = thinking_row.source_agent_id.clone();
                        message_row.content = Some(content);
                        message_row.mutable = false;

                        if let Err(e) = db.append_row(&mut message_row) {
                            tracing::error!("failed to create message.model row: {}", e);
                        }

                        // Mark the thinking.stream row as ephemeral (won't show in history)
                        if let Err(e) = db.set_row_ephemeral(&row_id, true) {
                            tracing::error!("failed to mark thinking row ephemeral: {}", e);
                        }
                    }
                }

                // Finalize the thinking row
                if let Err(e) = db.finalize_row(&row_id) {
                    tracing::error!("failed to finalize row: {}", e);
                }

                // Update status to idle for Lua HUD
                if let Some(ref lua_runtime) = lua_runtime {
                    let lua = lua_runtime.lock().await;
                    lua.tool_state().set_status(&model_name, Status::Idle);
                    lua.tool_state().mark_dirty("chat");
                }
            }
        }
    }
}
