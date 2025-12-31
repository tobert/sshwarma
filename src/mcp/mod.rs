//! MCP integration: client (to holler) and server (expose to Claude Code)
//!
//! Uses rmcp (official Rust MCP SDK) with streamable HTTP transport.

mod backoff;
mod client;
mod events;
mod manager;

// Re-export backoff types
pub use backoff::Backoff;

// Re-export event types
pub use events::{McpEvent, McpEventReceiver, McpEventSender};

// Re-export manager types (new managed approach)
pub use manager::{ConnectionState, ConnectionStatus, McpManager};

// Re-export common types from manager (these replace client types going forward)
pub use manager::{RigToolContext, ToolInfo, ToolResult};

// Re-export legacy client types for backwards compatibility during migration
pub use client::{ConnectionInfo, McpClients};
