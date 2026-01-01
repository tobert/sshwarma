//! Async MCP bridge for Lua
//!
//! Provides a way for Lua (sync) to initiate async MCP tool calls
//! and poll for results. Uses channels to communicate between
//! sync Lua callbacks and async MCP handlers.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, warn};

use crate::mcp::McpManager;

/// Request status for async MCP calls
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Complete,
    Error,
    Timeout,
}

impl RequestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestStatus::Pending => "pending",
            RequestStatus::Complete => "complete",
            RequestStatus::Error => "error",
            RequestStatus::Timeout => "timeout",
        }
    }
}

/// State of a pending or completed request
#[derive(Debug, Clone)]
pub struct RequestState {
    pub status: RequestStatus,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub created_at: Instant,
}

impl RequestState {
    fn new_pending() -> Self {
        Self {
            status: RequestStatus::Pending,
            result: None,
            error: None,
            created_at: Instant::now(),
        }
    }
}

/// MCP request to be processed asynchronously
#[derive(Debug)]
pub struct McpRequest {
    pub request_id: String,
    pub server: String,
    pub tool: String,
    pub args: Value,
}

/// Bridge between sync Lua and async MCP operations
///
/// Lua calls `mcp_call()` which queues a request and returns immediately
/// with a request ID. Lua then polls `mcp_result()` to check status.
#[derive(Clone)]
pub struct McpBridge {
    /// Pending requests: request_id -> RequestState
    requests: Arc<RwLock<HashMap<String, RequestState>>>,
    /// Channel to send requests to async handler
    request_tx: mpsc::Sender<McpRequest>,
    /// Request timeout (default 30s)
    timeout: Duration,
}

impl McpBridge {
    /// Create a new MCP bridge with a channel for async processing
    pub fn new(timeout: Duration) -> (Self, mpsc::Receiver<McpRequest>) {
        let (tx, rx) = mpsc::channel(64);
        let bridge = Self {
            requests: Arc::new(RwLock::new(HashMap::new())),
            request_tx: tx,
            timeout,
        };
        (bridge, rx)
    }

    /// Create with default 30s timeout
    pub fn with_defaults() -> (Self, mpsc::Receiver<McpRequest>) {
        Self::new(Duration::from_secs(30))
    }

    /// Get the timeout duration
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Get the requests map for the async handler
    pub fn requests(&self) -> Arc<RwLock<HashMap<String, RequestState>>> {
        self.requests.clone()
    }

    /// Called from Lua (sync) - queues request, returns ID immediately
    pub fn call(&self, server: &str, tool: &str, args: Value) -> String {
        let request_id = format!("req_{}", uuid::Uuid::new_v4().simple());

        // Insert pending state
        if let Ok(mut guard) = self.requests.try_write() {
            guard.insert(request_id.clone(), RequestState::new_pending());
        } else {
            warn!("Could not acquire write lock for request state");
        }

        // Queue request (non-blocking send)
        let request = McpRequest {
            request_id: request_id.clone(),
            server: server.to_string(),
            tool: tool.to_string(),
            args,
        };

        if let Err(e) = self.request_tx.try_send(request) {
            warn!("Failed to queue MCP request: {}", e);
            // Mark as error
            if let Ok(mut guard) = self.requests.try_write() {
                if let Some(state) = guard.get_mut(&request_id) {
                    state.status = RequestStatus::Error;
                    state.error = Some("request queue full".to_string());
                }
            }
        }

        debug!("Queued MCP request {} for {}:{}", request_id, server, tool);
        request_id
    }

    /// Called from Lua (sync) - checks result of a request
    ///
    /// Returns (result_or_error, status_string)
    pub fn result(&self, request_id: &str) -> (Option<Value>, &'static str) {
        if let Ok(guard) = self.requests.try_read() {
            match guard.get(request_id) {
                Some(state) => match state.status {
                    RequestStatus::Pending => (None, "pending"),
                    RequestStatus::Complete => (state.result.clone(), "complete"),
                    RequestStatus::Error => {
                        let error_val = state
                            .error
                            .as_ref()
                            .map(|e| serde_json::json!({"error": e}));
                        (error_val, "error")
                    }
                    RequestStatus::Timeout => (None, "timeout"),
                },
                None => (None, "unknown"),
            }
        } else {
            (None, "pending") // Can't get lock, assume still pending
        }
    }

    /// Clean up completed/old requests (call periodically)
    pub async fn cleanup_old_requests(&self, max_age: Duration) {
        let mut guard = self.requests.write().await;
        let now = Instant::now();
        guard.retain(|_id, state| {
            // Keep pending requests and recently completed ones
            state.status == RequestStatus::Pending || now.duration_since(state.created_at) < max_age
        });
    }
}

/// Spawn the async handler task that processes MCP requests
///
/// This should be spawned per-connection and will process requests
/// from the bridge's channel.
pub async fn mcp_request_handler(
    mut rx: mpsc::Receiver<McpRequest>,
    mcp_manager: Arc<McpManager>,
    requests: Arc<RwLock<HashMap<String, RequestState>>>,
    timeout: Duration,
) {
    debug!("MCP request handler started");

    while let Some(req) = rx.recv().await {
        let mcp = mcp_manager.clone();
        let reqs = requests.clone();
        let request_id = req.request_id.clone();

        debug!(
            "Processing MCP request {} for {}:{}",
            request_id, req.server, req.tool
        );

        // Spawn a task to handle this request
        tokio::spawn(async move {
            let result = tokio::time::timeout(timeout, async {
                mcp.call_tool(&req.tool, req.args.clone()).await
            })
            .await;

            // Update request state
            let mut guard = reqs.write().await;
            if let Some(state) = guard.get_mut(&request_id) {
                match result {
                    Ok(Ok(tool_result)) => {
                        state.status = RequestStatus::Complete;
                        // Try to parse as JSON, fall back to string
                        state.result = Some(
                            serde_json::from_str(&tool_result.content)
                                .unwrap_or_else(|_| serde_json::json!(tool_result.content)),
                        );
                        debug!("MCP request {} completed", request_id);
                    }
                    Ok(Err(e)) => {
                        state.status = RequestStatus::Error;
                        state.error = Some(e.to_string());
                        warn!("MCP request {} failed: {}", request_id, e);
                    }
                    Err(_) => {
                        state.status = RequestStatus::Timeout;
                        warn!("MCP request {} timed out", request_id);
                    }
                }
            }
        });
    }

    debug!("MCP request handler stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_status_as_str() {
        assert_eq!(RequestStatus::Pending.as_str(), "pending");
        assert_eq!(RequestStatus::Complete.as_str(), "complete");
        assert_eq!(RequestStatus::Error.as_str(), "error");
        assert_eq!(RequestStatus::Timeout.as_str(), "timeout");
    }

    #[tokio::test]
    async fn test_bridge_call_and_result() {
        let (bridge, _rx) = McpBridge::with_defaults();

        // Call returns a request ID
        let req_id = bridge.call("holler", "garden_status", serde_json::json!({}));
        assert!(req_id.starts_with("req_"));

        // Initial status is pending
        let (result, status) = bridge.result(&req_id);
        assert_eq!(status, "pending");
        assert!(result.is_none());
    }

    #[test]
    fn test_unknown_request() {
        let (bridge, _rx) = McpBridge::with_defaults();

        let (result, status) = bridge.result("nonexistent");
        assert_eq!(status, "unknown");
        assert!(result.is_none());
    }
}
