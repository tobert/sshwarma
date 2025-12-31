//! MCP integration: client (to holler) and server (expose to Claude Code)
//!
//! Uses rmcp (official Rust MCP SDK) with streamable HTTP transport.

mod backoff;
mod client;
mod events;

// Re-export backoff types
pub use backoff::Backoff;

// Re-export event types
pub use events::{McpEvent, McpEventReceiver, McpEventSender};

// Re-export client types for backwards compatibility
pub use client::{ConnectionInfo, McpClients, RigToolContext, ToolInfo, ToolResult};
