//! sshwarma - SSH-accessible collaborative space for humans and models
//!
//! A MUD-style REPL where users connect via SSH and collaborate with
//! AI models in shared rooms. Plain text is chat, /commands control
//! navigation and tools, @mentions address models.

use anyhow::{Context, Result};
use russh::server::Server as _;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use sshwarma::config::{Config, ModelsConfig};
use sshwarma::db::Database;
use sshwarma::llm::LlmClient;
use sshwarma::mcp::McpClients;
use sshwarma::mcp_server::{self, McpServerState};
use sshwarma::model::ModelRegistry;
use sshwarma::ssh::SshServer;
use sshwarma::state::SharedState;
use sshwarma::world::World;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("sshwarma=info".parse()?),
        )
        .init();

    let config = Config::default();
    info!(addr = %config.listen_addr, "starting sshwarma");

    // Load or generate host key
    let key = load_or_generate_host_key(&config)?;
    let russh_config = russh::server::Config {
        keys: vec![key],
        inactivity_timeout: Some(std::time::Duration::from_secs(20 * 60)),
        ..Default::default()
    };

    // Initialize database
    info!("opening database at {}", config.db_path);
    let db = Database::open(&config.db_path).context("failed to open database")?;

    // Check users
    let users = db.list_users()?;
    if users.is_empty() {
        if config.allow_open_registration {
            warn!("no users registered - running in open mode");
            warn!("use sshwarma-admin to add users");
        } else {
            anyhow::bail!("no users registered and open registration disabled");
        }
    } else {
        info!("{} users registered", users.len());
    }

    // Load models configuration
    let models_config = ModelsConfig::load(&config.models_config_path)
        .context("failed to load models config")?;

    // Initialize LLM client (set OLLAMA_HOST for local models)
    info!("initializing LLM client (ollama: {})", models_config.ollama_endpoint);
    let llm = LlmClient::with_ollama_endpoint(&models_config.ollama_endpoint)
        .context("failed to create LLM client")?;

    // Build model registry from config
    let models = ModelRegistry::from_config(&models_config);
    info!("{} models registered", models.list().len());

    // Load rooms from database
    let mut world = World::new();
    let saved_rooms = db.get_all_rooms()?;
    for room_info in &saved_rooms {
        world.create_room(room_info.name.clone());
        if let Some(room) = world.get_room_mut(&room_info.name) {
            room.description = room_info.description.clone();
        }
    }
    info!("{} rooms loaded from database", saved_rooms.len());

    // Wrap in Arc for sharing
    let world = Arc::new(RwLock::new(world));
    let db = Arc::new(db);
    let llm = Arc::new(llm);
    let models = Arc::new(models);
    let mcp = McpClients::new();

    let state = Arc::new(SharedState {
        world: world.clone(),
        db: db.clone(),
        config: config.clone(),
        llm: llm.clone(),
        models: models.clone(),
        mcp,
    });

    // Start MCP server for Claude Code
    if config.mcp_server_port > 0 {
        let mcp_state = Arc::new(McpServerState {
            world: world.clone(),
            db: db.clone(),
            llm: llm.clone(),
            models: models.clone(),
        });
        let _mcp_handle = mcp_server::start_mcp_server(config.mcp_server_port, mcp_state).await?;
        info!(port = config.mcp_server_port, "MCP server started");
    }

    // Start SSH server
    let mut server = SshServer { state };
    info!("listening on {}", config.listen_addr);
    server
        .run_on_address(Arc::new(russh_config), config.listen_addr)
        .await?;

    Ok(())
}

fn load_or_generate_host_key(config: &Config) -> Result<russh::keys::PrivateKey> {
    let key_path = std::path::Path::new(&config.host_key_path);

    if key_path.exists() {
        info!("loading host key from {}", config.host_key_path);
        Ok(russh::keys::decode_secret_key(
            &std::fs::read_to_string(&config.host_key_path)?,
            None,
        )?)
    } else {
        info!("generating new host key");
        let key = russh::keys::PrivateKey::random(
            &mut rand::thread_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .context("failed to generate key")?;

        std::fs::write(
            &config.host_key_path,
            key.to_openssh(russh::keys::ssh_key::LineEnding::LF)?,
        )?;

        Ok(key)
    }
}
