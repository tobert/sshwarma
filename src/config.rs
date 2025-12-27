//! Server configuration

use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Server configuration
#[derive(Clone)]
pub struct Config {
    /// SSH listen address
    pub listen_addr: SocketAddr,
    /// Path to server host key
    pub host_key_path: String,
    /// Path to sqlite database
    pub db_path: String,
    /// MCP server endpoints (holler, exa, etc.)
    pub mcp_endpoints: Vec<String>,
    /// Allow any key when no users registered (dev mode)
    pub allow_open_registration: bool,
    /// MCP server port for Claude Code (0 = disabled)
    pub mcp_server_port: u16,
    /// Path to models config file
    pub models_config_path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:2222".parse().unwrap(),
            host_key_path: "host_key".to_string(),
            db_path: "sshwarma.db".to_string(),
            mcp_endpoints: vec!["http://localhost:8080/mcp".to_string()],
            allow_open_registration: true,
            mcp_server_port: 2223,
            models_config_path: "models.toml".to_string(),
        }
    }
}

/// Models configuration file structure
#[derive(Debug, Deserialize)]
pub struct ModelsConfig {
    /// Default Ollama/llama.cpp endpoint
    #[serde(default = "default_ollama_endpoint")]
    pub ollama_endpoint: String,
    /// Model definitions
    #[serde(default)]
    pub models: Vec<ModelConfig>,
}

fn default_ollama_endpoint() -> String {
    "http://localhost:11434".to_string()
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            ollama_endpoint: default_ollama_endpoint(),
            models: vec![],
        }
    }
}

/// Single model configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    /// Short name for @mentions (e.g., "qwen-8b")
    pub name: String,
    /// Display name (e.g., "Qwen3-VL-8B-Instruct")
    pub display: String,
    /// Model identifier for the backend
    pub model: String,
    /// Backend type: ollama, openai, anthropic, gemini
    pub backend: String,
    /// Optional custom endpoint (overrides default)
    pub endpoint: Option<String>,
    /// Whether model is enabled (default true)
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Custom system prompt for this model
    pub system_prompt: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl ModelsConfig {
    /// Load models config from a TOML file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            tracing::warn!("models config not found at {}, using defaults", path.display());
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        let config: ModelsConfig = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;

        tracing::info!(
            "loaded {} models from {}",
            config.models.len(),
            path.display()
        );

        Ok(config)
    }
}
