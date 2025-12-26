//! MCP tool name completion

use std::sync::Arc;

use crate::state::SharedState;

use super::Completion;

pub struct ToolCompleter;

impl ToolCompleter {
    /// Get tool completions from MCP clients
    pub async fn complete(state: &Arc<SharedState>) -> Vec<Completion> {
        let tools = state.mcp.list_tools().await;

        tools
            .into_iter()
            .map(|tool| {
                let desc = tool.description.chars().take(30).collect::<String>();
                Completion {
                    text: tool.name.clone(),
                    label: format!("{:<20} {}", tool.name, desc),
                    score: 0,
                }
            })
            .collect()
    }
}
