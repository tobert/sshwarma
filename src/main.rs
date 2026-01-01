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
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use sshwarma::config::{Config, ModelsConfig};
use sshwarma::db::Database;
use sshwarma::llm::LlmClient;
use sshwarma::mcp::McpManager;
use sshwarma::mcp_server::{self, McpServerState};
use sshwarma::model::ModelRegistry;
use sshwarma::paths;
use sshwarma::ssh::SshServer;
use sshwarma::state::SharedState;
use sshwarma::world::World;

#[tokio::main]
async fn main() -> Result<()> {
    init_telemetry()?;

    // Ensure XDG directories exist
    paths::ensure_dirs().context("failed to create directories")?;

    let config = Config::from_env();
    info!(addr = %config.listen_addr, "starting sshwarma");
    paths::log_paths();

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
    let models_config =
        ModelsConfig::load(&config.models_config_path).context("failed to load models config")?;

    // Initialize LLM client (set OLLAMA_HOST for local models)
    info!(
        "initializing LLM client (ollama: {})",
        models_config.ollama_endpoint
    );
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
    let mcp = Arc::new(McpManager::new());

    let state = Arc::new(SharedState {
        world: world.clone(),
        db: db.clone(),
        config: config.clone(),
        llm: llm.clone(),
        models: models.clone(),
        mcp,
    });

    // Run Lua startup script (can configure MCP connections, etc.)
    {
        use sshwarma::lua::LuaRuntime;

        info!("creating Lua runtime for startup script");
        let lua = LuaRuntime::new().context("failed to create Lua runtime for startup")?;
        info!("setting shared state for startup script");
        lua.tool_state().set_shared_state(Some(state.clone()));

        info!("running startup script...");
        match lua.run_startup_script() {
            Ok(true) => info!("startup script executed successfully"),
            Ok(false) => info!("no startup script found (create ~/.config/sshwarma/startup.lua)"),
            Err(e) => warn!("startup script failed: {}", e),
        }
        info!("startup script phase complete");
    }

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

/// Initialize telemetry with optional OTLP export
///
/// When OTEL_EXPORTER_OTLP_ENDPOINT is set, traces and metrics are exported
/// via OTLP (e.g., to Jaeger, Grafana Tempo, or any OTLP-compatible backend).
/// Otherwise, only console logging via tracing-subscriber is enabled.
fn init_telemetry() -> Result<()> {
    use tracing_subscriber::EnvFilter;

    // Console logging layer (always enabled)
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(false);

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("sshwarma=info"));

    // Optional OTLP export when endpoint is configured
    let otel_layer = if let Ok(endpoint) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        use opentelemetry::trace::TracerProvider;
        use opentelemetry_otlp::{SpanExporter, WithExportConfig};

        let service_name =
            std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "sshwarma".to_string());

        let resource = opentelemetry_sdk::Resource::new(vec![opentelemetry::KeyValue::new(
            "service.name",
            service_name.clone(),
        )]);

        // Build OTLP span exporter
        let exporter = SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .build()
            .context("failed to build OTLP exporter")?;

        // Create tracer provider with batch export
        let tracer_provider = opentelemetry_sdk::trace::TracerProvider::builder()
            .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
            .with_resource(resource.clone())
            .build();

        let tracer = tracer_provider.tracer("sshwarma");

        // Set global tracer provider
        opentelemetry::global::set_tracer_provider(tracer_provider);

        // Initialize metrics provider with OTLP export
        use opentelemetry_otlp::MetricExporter;

        let metrics_exporter = MetricExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .build()
            .context("failed to build OTLP metrics exporter")?;

        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(
            metrics_exporter,
            opentelemetry_sdk::runtime::Tokio,
        )
        .with_interval(std::time::Duration::from_secs(10))
        .build();

        let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(resource)
            .build();

        opentelemetry::global::set_meter_provider(meter_provider);

        eprintln!("OTLP export enabled: {} -> {}", service_name, endpoint);
        Some(tracing_opentelemetry::layer().with_tracer(tracer))
    } else {
        None
    };

    // Build the subscriber
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(otel_layer)
        .init();

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
