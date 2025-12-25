//! MCP integration: client (to holler, exa) and server (expose to Claude Code)

use anyhow::Result;
use std::sync::Arc;

/// MCP client for connecting to tool providers (holler, exa, etc.)
pub struct McpClients {
    clients: Vec<McpConnection>,
}

struct McpConnection {
    name: String,
    endpoint: String,
    // client: baton::client::McpClient, // TODO: uncomment when wired up
}

impl McpClients {
    pub fn new() -> Self {
        Self {
            clients: Vec::new(),
        }
    }

    /// Connect to an MCP server
    pub async fn connect(&mut self, name: &str, endpoint: &str) -> Result<()> {
        // TODO: Initialize baton client, call initialize
        self.clients.push(McpConnection {
            name: name.to_string(),
            endpoint: endpoint.to_string(),
        });
        Ok(())
    }

    /// List all available tools across connected MCPs
    pub async fn list_tools(&self) -> Result<Vec<ToolInfo>> {
        // TODO: Aggregate tools from all clients
        Ok(Vec::new())
    }

    /// Call a tool by name, routing to the appropriate MCP
    pub async fn call_tool(&self, name: &str, args: serde_json::Value) -> Result<ToolResult> {
        // TODO: Find which client has this tool, call it
        Err(anyhow::anyhow!("tool {} not found", name))
    }
}

/// Tool metadata
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub source: String, // Which MCP server provides this
}

/// Result from a tool call
#[derive(Debug)]
pub struct ToolResult {
    pub content: String,
    pub artifact_id: Option<String>,
}

// ============================================================================
// MCP Server (expose sshwarma to Claude Code)
// ============================================================================

/// Build MCP router for sshwarma's own tools
pub fn build_mcp_server() -> Result<()> {
    // TODO: Implement baton::Handler for sshwarma tools
    // Tools: rooms, join, leave, create, look, examine, who, history,
    //        say, tell, get, drop, inventory, play, stop, tools, run
    Ok(())
}
