//! MCP integration: client (to holler) and server (expose to Claude Code)
//!
//! Uses rmcp (official Rust MCP SDK) with streamable HTTP transport.

use anyhow::{Context, Result};
use rmcp::{
    RoleClient,
    model::{CallToolRequestParam, CallToolResult as RmcpCallToolResult, Tool},
    service::{RunningService, ServiceExt},
    transport::StreamableHttpClientTransport,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Type alias for the running MCP client service
type McpClientService = RunningService<RoleClient, ()>;

/// MCP client for connecting to tool providers (holler, etc.)
pub struct McpClients {
    clients: RwLock<HashMap<String, McpConnection>>,
}

/// Active MCP connection with cached tools
struct McpConnection {
    endpoint: String,
    service: Arc<McpClientService>,
    tools: Vec<Tool>,
}

impl McpClients {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    /// Connect to an MCP server via streamable HTTP
    pub async fn connect(&self, name: &str, endpoint: &str) -> Result<()> {
        info!("connecting to MCP server '{}' at {}", name, endpoint);

        // Create transport using reqwest
        let transport = StreamableHttpClientTransport::from_uri(endpoint);

        // Connect and initialize - () implements ClientHandler with defaults
        let service = ().serve(transport).await
            .map_err(|e| anyhow::anyhow!("failed to connect: {:?}", e))?;

        // Get server info
        let peer_info = service.peer_info();
        info!("connected to MCP server: {:?}", peer_info);

        // List available tools
        let tools_result = service.list_tools(Default::default()).await
            .context("failed to list tools")?;

        info!("{} tools available from '{}'", tools_result.tools.len(), name);
        for tool in &tools_result.tools {
            debug!("  - {}: {:?}", tool.name, tool.description);
        }

        // Store connection
        let connection = McpConnection {
            endpoint: endpoint.to_string(),
            service: Arc::new(service),
            tools: tools_result.tools,
        };

        self.clients.write().await.insert(name.to_string(), connection);
        Ok(())
    }

    /// Disconnect from an MCP server
    pub async fn disconnect(&self, name: &str) -> Result<bool> {
        if let Some(conn) = self.clients.write().await.remove(name) {
            // Use cancellation token to trigger graceful shutdown
            conn.service.cancellation_token().cancel();
            info!("disconnected from MCP server '{}'", name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// List all available tools across connected MCPs
    pub async fn list_tools(&self) -> Vec<ToolInfo> {
        let clients = self.clients.read().await;
        let mut all_tools = Vec::new();

        for (source, conn) in clients.iter() {
            for tool in &conn.tools {
                all_tools.push(ToolInfo {
                    name: tool.name.to_string(),
                    description: tool.description.clone().unwrap_or_default().to_string(),
                    source: source.clone(),
                });
            }
        }

        all_tools
    }

    /// Refresh tool list from a specific MCP
    pub async fn refresh_tools(&self, name: &str) -> Result<()> {
        let mut clients = self.clients.write().await;
        if let Some(conn) = clients.get_mut(name) {
            let tools_result = conn.service.list_tools(Default::default()).await
                .context("failed to list tools")?;
            info!("refreshed {} tools from '{}'", tools_result.tools.len(), name);
            conn.tools = tools_result.tools;
            Ok(())
        } else {
            Err(anyhow::anyhow!("MCP '{}' not connected", name))
        }
    }

    /// Call a tool by name, routing to the appropriate MCP
    pub async fn call_tool(&self, name: &str, args: serde_json::Value) -> Result<ToolResult> {
        let clients = self.clients.read().await;

        // Find which client has this tool
        for (source, conn) in clients.iter() {
            if conn.tools.iter().any(|t| t.name == name) {
                debug!("calling tool '{}' on MCP '{}'", name, source);

                let result: RmcpCallToolResult = conn.service.call_tool(CallToolRequestParam {
                    name: name.to_string().into(),
                    arguments: args.as_object().cloned(),
                }).await.context("tool call failed")?;

                // Extract text content from result using as_text()
                let content = result.content.iter()
                    .filter_map(|c| c.as_text().map(|t| t.text.to_string()))
                    .collect::<Vec<_>>()
                    .join("\n");

                return Ok(ToolResult {
                    content,
                    is_error: result.is_error.unwrap_or(false),
                    source: source.clone(),
                });
            }
        }

        Err(anyhow::anyhow!("tool '{}' not found in any connected MCP", name))
    }

    /// List connected MCP servers
    pub async fn list_connections(&self) -> Vec<ConnectionInfo> {
        self.clients.read().await.iter()
            .map(|(name, conn)| ConnectionInfo {
                name: name.clone(),
                endpoint: conn.endpoint.clone(),
                tool_count: conn.tools.len(),
            })
            .collect()
    }
}

impl Default for McpClients {
    fn default() -> Self {
        Self::new()
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
    pub is_error: bool,
    pub source: String,
}

/// Connection info for display
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub name: String,
    pub endpoint: String,
    pub tool_count: usize,
}

// ============================================================================
// MCP Server (expose sshwarma to Claude Code)
// ============================================================================

// TODO: Implement sshwarma's own MCP server
// Tools to expose:
// - rooms: list available rooms
// - join: join a room
// - leave: leave current room
// - look: describe current room
// - who: list users in room
// - say: send a message
// - history: get message history
// - inventory: list items
// - tools: list available tools from connected MCPs
// - run: execute a tool

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_clients_new() {
        let clients = McpClients::new();
        // Just verify it constructs without panic
        let _ = clients;
    }
}
