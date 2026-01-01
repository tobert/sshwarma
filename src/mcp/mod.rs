//! MCP integration: client (to holler) and server (expose to Claude Code)
//!
//! Uses rmcp (official Rust MCP SDK) with streamable HTTP transport.

mod backoff;
mod events;
mod manager;

// Re-export backoff types
pub use backoff::Backoff;

// Re-export event types
pub use events::{McpEvent, McpEventReceiver, McpEventSender};

// Re-export manager types
pub use manager::{ConnectionState, ConnectionStatus, McpManager};

// Re-export common types from manager
pub use manager::{RigToolContext, ToolInfo, ToolResult};
