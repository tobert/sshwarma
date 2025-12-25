//! Shared server state

use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::db::Database;
use crate::llm::LlmClient;
use crate::mcp::McpClients;
use crate::model::ModelRegistry;
use crate::world::World;

/// The shared world state accessible by both SSH and MCP servers
pub struct SharedState {
    pub world: Arc<RwLock<World>>,
    pub db: Arc<Database>,
    pub config: Config,
    pub llm: Arc<LlmClient>,
    pub models: Arc<ModelRegistry>,
    pub mcp: McpClients,
}
