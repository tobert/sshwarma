//! MCP connection manager with automatic retry and event broadcasting.
//!
//! Manages the lifecycle of MCP connections:
//! - Non-blocking add/remove from Lua control plane
//! - Background tasks handle connection with exponential backoff
//! - Events broadcast to HUD, logs, etc.

use super::{Backoff, McpEvent, McpEventSender};
use anyhow::{Context, Result};
use rmcp::{
    model::{CallToolRequestParam, CallToolResult as RmcpCallToolResult, Tool},
    service::{RunningService, ServiceExt},
    transport::StreamableHttpClientTransport,
    RoleClient,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};

/// Type alias for the running MCP client service.
type McpService = RunningService<RoleClient, ()>;

/// Connection state machine.
#[derive(Debug, Clone)]
pub enum ConnectionState {
    /// Initial state, connection task spawned.
    Connecting,
    /// Successfully connected.
    Connected {
        /// Number of tools available.
        tool_count: usize,
    },
    /// Failed, retrying with backoff (infinite retry).
    Reconnecting {
        /// Current retry attempt number.
        attempt: u32,
        /// Error from last failed attempt.
        last_error: String,
    },
}

impl ConnectionState {
    /// Get the state name as a string for display.
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectionState::Connecting => "connecting",
            ConnectionState::Connected { .. } => "connected",
            ConnectionState::Reconnecting { .. } => "reconnecting",
        }
    }
}

/// Internal connection tracking.
struct ManagedConnection {
    /// Endpoint URL.
    endpoint: String,
    /// Current state.
    state: ConnectionState,
    /// Active service (when connected).
    service: Option<Arc<McpService>>,
    /// Cached tools.
    tools: Vec<Tool>,
    /// Total tool calls made.
    call_count: u64,
    /// Most recently called tool name.
    last_tool: Option<String>,
    /// Cancellation token for background task.
    cancel: CancellationToken,
}

/// Connection status for API responses.
#[derive(Debug, Clone)]
pub struct ConnectionStatus {
    /// Connection name.
    pub name: String,
    /// Endpoint URL.
    pub endpoint: String,
    /// State as string ("connecting", "connected", "reconnecting").
    pub state: String,
    /// Number of available tools.
    pub tool_count: usize,
    /// Error message if reconnecting.
    pub error: Option<String>,
    /// Retry attempt if reconnecting.
    pub attempt: Option<u32>,
    /// Total tool calls made.
    pub call_count: u64,
    /// Most recently called tool.
    pub last_tool: Option<String>,
}

/// Tools and peer for use with rig agents.
pub struct RigToolContext {
    /// List of tools paired with the connection peer that handles them.
    pub tools: Vec<(Tool, rmcp::service::ServerSink)>,
}

/// Result from a tool call.
#[derive(Debug)]
pub struct ToolResult {
    /// Response content.
    pub content: String,
    /// Whether the tool reported an error.
    pub is_error: bool,
    /// Which MCP server handled this.
    pub source: String,
}

/// Tool metadata.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// Which MCP server provides this.
    pub source: String,
}

/// MCP connection manager.
///
/// Manages MCP connections with automatic retry and event broadcasting.
/// The control plane (add, remove, status, list) is non-blocking.
/// Connection lifecycle runs in background tasks.
pub struct McpManager {
    /// Desired connections (name → endpoint).
    desired: RwLock<HashMap<String, String>>,
    /// Actual connections (name → state).
    connections: Arc<RwLock<HashMap<String, ManagedConnection>>>,
    /// Event broadcaster.
    events: McpEventSender,
}

impl McpManager {
    /// Create a new manager with default event sender.
    pub fn new() -> Self {
        Self {
            desired: RwLock::new(HashMap::new()),
            connections: Arc::new(RwLock::new(HashMap::new())),
            events: McpEventSender::default(),
        }
    }

    /// Create a new manager with an external event sender.
    pub fn with_events(sender: McpEventSender) -> Self {
        Self {
            desired: RwLock::new(HashMap::new()),
            connections: Arc::new(RwLock::new(HashMap::new())),
            events: sender,
        }
    }

    /// Subscribe to connection events.
    pub fn subscribe(&self) -> super::McpEventReceiver {
        self.events.subscribe()
    }

    // ========================================================================
    // Control Plane (non-blocking, called by Lua)
    // ========================================================================

    /// Add a connection to the desired state and spawn a background task.
    ///
    /// This is non-blocking and idempotent:
    /// - Same name + URL: no-op
    /// - Same name + different URL: update endpoint and reconnect
    /// - New name: spawn connection task
    #[instrument(skip(self), fields(mcp.server = %name, mcp.endpoint = %endpoint))]
    pub fn add(&self, name: &str, endpoint: &str) {
        info!("adding MCP connection");

        // Use block_in_place to safely access RwLock from sync context
        let should_spawn = tokio::task::block_in_place(|| {
            let mut desired = self.desired.blocking_write();
            let mut connections = self.connections.blocking_write();

            // Check if already exists with same endpoint
            if let Some(existing) = desired.get(name) {
                if existing == endpoint {
                    debug!("connection already exists with same endpoint");
                    return false;
                }
                // Different endpoint - need to reconnect
                info!("endpoint changed, reconnecting");
                if let Some(conn) = connections.remove(name) {
                    conn.cancel.cancel();
                }
            }

            // Update desired state
            desired.insert(name.to_string(), endpoint.to_string());

            // Create cancellation token for this connection
            let cancel = CancellationToken::new();

            // Initialize connection entry
            connections.insert(
                name.to_string(),
                ManagedConnection {
                    endpoint: endpoint.to_string(),
                    state: ConnectionState::Connecting,
                    service: None,
                    tools: Vec::new(),
                    call_count: 0,
                    last_tool: None,
                    cancel: cancel.clone(),
                },
            );

            true
        });

        if should_spawn {
            // Spawn background connection task
            let name = name.to_string();
            let endpoint = endpoint.to_string();
            let connections = self.connections.clone();
            let events = self.events.clone();
            let cancel = tokio::task::block_in_place(|| {
                self.connections
                    .blocking_read()
                    .get(&name)
                    .map(|c| c.cancel.clone())
                    .unwrap()
            });

            tokio::spawn(connection_loop(name, endpoint, connections, events, cancel));
        }
    }

    /// Remove a connection from the desired state.
    ///
    /// Cancels the background task and emits a Removed event.
    /// Returns true if the connection was present.
    #[instrument(skip(self), fields(mcp.server = %name))]
    pub fn remove(&self, name: &str) -> bool {
        info!("removing MCP connection");

        tokio::task::block_in_place(|| {
            let mut desired = self.desired.blocking_write();
            let mut connections = self.connections.blocking_write();

            if desired.remove(name).is_some() {
                if let Some(conn) = connections.remove(name) {
                    // Cancel background task - it will emit Removed event
                    conn.cancel.cancel();
                    // Also gracefully close service if connected
                    if let Some(service) = conn.service {
                        service.cancellation_token().cancel();
                    }
                }
                true
            } else {
                false
            }
        })
    }

    /// Get status of a single connection.
    pub fn status(&self, name: &str) -> Option<ConnectionStatus> {
        tokio::task::block_in_place(|| {
            let connections = self.connections.blocking_read();
            connections.get(name).map(|conn| ConnectionStatus {
                name: name.to_string(),
                endpoint: conn.endpoint.clone(),
                state: conn.state.as_str().to_string(),
                tool_count: conn.tools.len(),
                error: match &conn.state {
                    ConnectionState::Reconnecting { last_error, .. } => Some(last_error.clone()),
                    _ => None,
                },
                attempt: match &conn.state {
                    ConnectionState::Reconnecting { attempt, .. } => Some(*attempt),
                    _ => None,
                },
                call_count: conn.call_count,
                last_tool: conn.last_tool.clone(),
            })
        })
    }

    /// List all connections with their status.
    pub fn list(&self) -> Vec<ConnectionStatus> {
        tokio::task::block_in_place(|| {
            let connections = self.connections.blocking_read();
            connections
                .iter()
                .map(|(name, conn)| ConnectionStatus {
                    name: name.clone(),
                    endpoint: conn.endpoint.clone(),
                    state: conn.state.as_str().to_string(),
                    tool_count: conn.tools.len(),
                    error: match &conn.state {
                        ConnectionState::Reconnecting { last_error, .. } => Some(last_error.clone()),
                        _ => None,
                    },
                    attempt: match &conn.state {
                        ConnectionState::Reconnecting { attempt, .. } => Some(*attempt),
                        _ => None,
                    },
                    call_count: conn.call_count,
                    last_tool: conn.last_tool.clone(),
                })
                .collect()
        })
    }

    // ========================================================================
    // Data Plane (called by existing tool logic)
    // ========================================================================

    /// Call a tool by name, routing to the appropriate MCP.
    pub async fn call_tool(&self, name: &str, args: serde_json::Value) -> Result<ToolResult> {
        // Find which connection has this tool
        let (source, service) = {
            let connections = self.connections.read().await;
            let mut found = None;
            for (src, conn) in connections.iter() {
                if conn.tools.iter().any(|t| t.name == name) {
                    if let Some(svc) = &conn.service {
                        found = Some((src.clone(), svc.clone()));
                        break;
                    }
                }
            }
            found.ok_or_else(|| {
                anyhow::anyhow!("tool '{}' not found in any connected MCP", name)
            })?
        };

        debug!(tool = %name, mcp = %source, "calling tool");

        let result: RmcpCallToolResult = service
            .call_tool(CallToolRequestParam {
                name: name.to_string().into(),
                arguments: args.as_object().cloned(),
            })
            .await
            .context("tool call failed")?;

        // Update call stats
        {
            let mut connections = self.connections.write().await;
            if let Some(conn) = connections.get_mut(&source) {
                conn.call_count += 1;
                conn.last_tool = Some(name.to_string());
            }
        }

        // Extract text content
        let content = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.to_string()))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolResult {
            content,
            is_error: result.is_error.unwrap_or(false),
            source,
        })
    }

    /// List all available tools across connected MCPs.
    pub async fn list_tools(&self) -> Vec<ToolInfo> {
        let connections = self.connections.read().await;
        let mut all_tools = Vec::new();

        for (source, conn) in connections.iter() {
            // Only include tools from connected MCPs
            if matches!(conn.state, ConnectionState::Connected { .. }) {
                for tool in &conn.tools {
                    all_tools.push(ToolInfo {
                        name: tool.name.to_string(),
                        description: tool.description.clone().unwrap_or_default().to_string(),
                        source: source.clone(),
                    });
                }
            }
        }

        all_tools
    }

    /// Refresh tool list from a specific MCP.
    pub async fn refresh_tools(&self, name: &str) -> Result<()> {
        let service = {
            let connections = self.connections.read().await;
            connections
                .get(name)
                .and_then(|c| c.service.clone())
                .ok_or_else(|| anyhow::anyhow!("MCP '{}' not connected", name))?
        };

        let tools_result = service
            .list_tools(Default::default())
            .await
            .context("failed to list tools")?;

        let tool_count = tools_result.tools.len();
        info!(mcp = %name, tool_count, "refreshed tools");

        {
            let mut connections = self.connections.write().await;
            if let Some(conn) = connections.get_mut(name) {
                conn.tools = tools_result.tools;
                conn.state = ConnectionState::Connected { tool_count };
            }
        }

        self.events.send(McpEvent::ToolsRefreshed {
            name: name.to_string(),
            tool_count,
        });

        Ok(())
    }

    /// Get tools and peer for rig agent integration.
    ///
    /// Returns all tools from all connected MCPs, each paired with its correct peer.
    /// Returns None if no MCPs are connected.
    pub async fn rig_tools(&self) -> Option<RigToolContext> {
        let connections = self.connections.read().await;

        let mut tools_with_peers = Vec::new();

        for conn in connections.values() {
            if let Some(service) = &conn.service {
                let peer = service.peer().to_owned();
                for tool in &conn.tools {
                    tools_with_peers.push((tool.clone(), peer.clone()));
                }
            }
        }

        if tools_with_peers.is_empty() {
            None
        } else {
            Some(RigToolContext {
                tools: tools_with_peers,
            })
        }
    }

    /// Get tools and peer for a specific MCP connection.
    pub async fn rig_tools_for(&self, name: &str) -> Option<RigToolContext> {
        let connections = self.connections.read().await;
        let conn = connections.get(name)?;
        let service = conn.service.as_ref()?;

        let peer = service.peer().to_owned();
        let tools_with_peers = conn.tools.iter().map(|t| (t.clone(), peer.clone())).collect();

        Some(RigToolContext {
            tools: tools_with_peers,
        })
    }

    /// List connected MCP servers (for backwards compatibility).
    pub async fn list_connections(&self) -> Vec<ConnectionStatus> {
        self.list()
    }

    /// Wait for a connection to reach Connected state.
    ///
    /// Useful for tests that need to ensure a connection is ready before proceeding.
    /// Polls the connection status at intervals until connected or timeout.
    pub async fn wait_for_connected(
        &self,
        name: &str,
        timeout: std::time::Duration,
    ) -> Result<()> {
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_millis(50);

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!(
                    "timeout waiting for MCP '{}' to connect",
                    name
                ));
            }

            if let Some(status) = self.status(name) {
                if status.state == "connected" {
                    return Ok(());
                }
                if let Some(error) = status.error {
                    // Still reconnecting, but log the error
                    debug!(mcp = %name, error = %error, "MCP reconnecting");
                }
            } else {
                return Err(anyhow::anyhow!("MCP '{}' not found", name));
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Background Connection Loop
// ============================================================================

/// Background task that manages a single MCP connection.
///
/// Connects with exponential backoff, monitors for disconnection,
/// and emits events on state changes.
#[instrument(
    name = "mcp.connection_loop",
    skip_all,
    fields(mcp.server = %name, mcp.endpoint = %endpoint)
)]
async fn connection_loop(
    name: String,
    endpoint: String,
    connections: Arc<RwLock<HashMap<String, ManagedConnection>>>,
    events: McpEventSender,
    cancel: CancellationToken,
) {
    let mut backoff = Backoff::new();

    loop {
        // Check if cancelled (user called mcp_remove)
        if cancel.is_cancelled() {
            events.send(McpEvent::Removed { name: name.clone() });
            // Clean up connection entry
            connections.write().await.remove(&name);
            break;
        }

        events.send(McpEvent::Connecting {
            name: name.clone(),
            endpoint: endpoint.clone(),
        });

        info!("attempting connection");

        match connect(&endpoint).await {
            Ok((service, tools)) => {
                let tool_count = tools.len();
                info!(tool_count, "connection established");

                events.send(McpEvent::Connected {
                    name: name.clone(),
                    endpoint: endpoint.clone(),
                    tool_count,
                });
                backoff.reset();

                // Store connection
                let service = Arc::new(service);
                {
                    let mut conns = connections.write().await;
                    if let Some(conn) = conns.get_mut(&name) {
                        conn.state = ConnectionState::Connected { tool_count };
                        conn.service = Some(service);
                        conn.tools = tools;
                    }
                }

                // Wait for user cancellation
                // Note: Service disconnection is detected on next operation failure
                cancel.cancelled().await;
                info!("connection cancelled by user");
                events.send(McpEvent::Removed { name: name.clone() });
                connections.write().await.remove(&name);
                break;
            }
            Err(e) => {
                let delay = backoff.next_delay();
                let attempt = backoff.attempt();
                let error = e.to_string();

                warn!(attempt, delay_ms = delay.as_millis(), error = %e, "connection failed, will retry");

                events.send(McpEvent::Reconnecting {
                    name: name.clone(),
                    attempt,
                    delay_ms: delay.as_millis() as u64,
                    error: error.clone(),
                });

                {
                    let mut conns = connections.write().await;
                    if let Some(conn) = conns.get_mut(&name) {
                        conn.state = ConnectionState::Reconnecting {
                            attempt,
                            last_error: error,
                        };
                        conn.service = None;
                    }
                }

                // Wait with cancellation check
                tokio::select! {
                    _ = tokio::time::sleep(delay) => continue,
                    _ = cancel.cancelled() => {
                        info!("connection cancelled during backoff");
                        events.send(McpEvent::Removed { name: name.clone() });
                        connections.write().await.remove(&name);
                        break;
                    }
                }
            }
        }
    }
}

/// Attempt to connect to an MCP server.
async fn connect(endpoint: &str) -> Result<(McpService, Vec<Tool>)> {
    let transport = StreamableHttpClientTransport::from_uri(endpoint);
    let service = ()
        .serve(transport)
        .await
        .map_err(|e| anyhow::anyhow!("failed to connect: {:?}", e))?;

    let tools_result = service
        .list_tools(Default::default())
        .await
        .context("failed to list tools")?;

    Ok((service, tools_result.tools))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_state_as_str() {
        assert_eq!(ConnectionState::Connecting.as_str(), "connecting");
        assert_eq!(
            ConnectionState::Connected { tool_count: 5 }.as_str(),
            "connected"
        );
        assert_eq!(
            ConnectionState::Reconnecting {
                attempt: 1,
                last_error: "err".into()
            }
            .as_str(),
            "reconnecting"
        );
    }

    #[test]
    fn test_manager_new() {
        let manager = McpManager::new();
        let list = manager.list();
        assert!(list.is_empty());
    }

    #[test]
    fn test_manager_default() {
        let manager = McpManager::default();
        let list = manager.list();
        assert!(list.is_empty());
    }
}
