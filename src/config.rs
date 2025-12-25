//! Server configuration

use std::net::SocketAddr;

/// Server configuration
#[derive(Clone)]
pub struct Config {
    /// SSH listen address
    pub listen_addr: SocketAddr,
    /// Path to server host key
    pub host_key_path: String,
    /// Path to sqlite database
    pub db_path: String,
    /// llama.cpp endpoint
    pub llm_endpoint: String,
    /// MCP server endpoints (holler, exa, etc.)
    pub mcp_endpoints: Vec<String>,
    /// Allow any key when no users registered (dev mode)
    pub allow_open_registration: bool,
    /// MCP server port for Claude Code (0 = disabled)
    pub mcp_server_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:2222".parse().unwrap(),
            host_key_path: "host_key".to_string(),
            db_path: "sshwarma.db".to_string(),
            llm_endpoint: "http://localhost:2020".to_string(),
            mcp_endpoints: vec!["http://localhost:8080/mcp".to_string()],
            allow_open_registration: true,
            mcp_server_port: 2223,
        }
    }
}
