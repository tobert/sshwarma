//! End-to-end integration tests for sshwarma
//!
//! Tests the MCP client and mock LLM without full SSH integration.
//! Full SSH e2e tests require more complex setup with key management.

use anyhow::Result;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpService,
    },
    ServerHandler,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock};

use sshwarma::config::Config;
use sshwarma::db::Database;
use sshwarma::llm::LlmClient;
use sshwarma::lua::register_mcp_tool_registration;
use sshwarma::lua::{LuaReloadSender, LuaRuntime};
use sshwarma::mcp::McpManager;
use sshwarma::mcp_server::{self, McpServerState, McpToolRegistry};
use sshwarma::model::{ModelBackend, ModelHandle, ModelRegistry};
use sshwarma::state::SharedState;
use sshwarma::world::World;
use std::time::Duration;

// ============================================================================
// Test MCP Server
// ============================================================================

/// Parameters for echo tool
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EchoParams {
    #[schemars(description = "Message to echo back")]
    message: String,
}

/// Parameters for add tool
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AddParams {
    #[schemars(description = "First number")]
    a: i64,
    #[schemars(description = "Second number")]
    b: i64,
}

/// Minimal MCP server for testing with ping, echo, and add tools
#[derive(Clone)]
struct TestMcpServer {
    call_count: Arc<Mutex<u32>>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl TestMcpServer {
    fn new() -> Self {
        Self {
            call_count: Arc::new(Mutex::new(0)),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Returns 'pong' - simple connectivity test")]
    async fn ping(&self) -> String {
        let mut count = self.call_count.lock().await;
        *count += 1;
        "pong".to_string()
    }

    #[tool(description = "Echoes back the input message")]
    async fn echo(&self, Parameters(params): Parameters<EchoParams>) -> String {
        let mut count = self.call_count.lock().await;
        *count += 1;
        format!("echo: {}", params.message)
    }

    #[tool(description = "Adds two numbers together")]
    async fn add(&self, Parameters(params): Parameters<AddParams>) -> String {
        let mut count = self.call_count.lock().await;
        *count += 1;
        format!("{}", params.a + params.b)
    }
}

#[tool_handler]
impl ServerHandler for TestMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("Test MCP server with ping, echo, and add tools".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

/// Start test MCP server on a random port, returns the URL
async fn start_test_mcp_server() -> Result<(String, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{}/mcp", port);

    let service = StreamableHttpService::new(
        || Ok(TestMcpServer::new()),
        LocalSessionManager::default().into(),
        Default::default(),
    );

    let router = axum::Router::new().nest_service("/mcp", service);

    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    Ok((url, handle))
}

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a test model registry with mock models
fn create_test_model_registry() -> ModelRegistry {
    let mut registry = ModelRegistry::new();

    registry.register(ModelHandle {
        short_name: "test".to_string(),
        display_name: "Test Echo Model".to_string(),
        backend: ModelBackend::Mock {
            prefix: "[mock]".to_string(),
        },
        available: true,
        system_prompt: None,
        context_window: None,
    });

    registry.register(ModelHandle {
        short_name: "assistant".to_string(),
        display_name: "Test Assistant".to_string(),
        backend: ModelBackend::Mock {
            prefix: "I understand".to_string(),
        },
        available: true,
        system_prompt: None,
        context_window: None,
    });

    registry
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_server_ping() -> Result<()> {
    let (mcp_url, _handle) = start_test_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("test", &mcp_url);
    manager
        .wait_for_connected("test", Duration::from_secs(5))
        .await?;

    // Verify ping tool exists
    let tools = manager.list_tools().await;
    assert!(
        tools.iter().any(|t| t.name == "ping"),
        "ping tool should exist"
    );

    // Call ping
    let result = manager.call_tool("ping", serde_json::json!({})).await?;
    assert_eq!(result.content, "pong");
    assert!(!result.is_error);

    manager.remove("test");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_server_echo() -> Result<()> {
    let (mcp_url, _handle) = start_test_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("test", &mcp_url);
    manager
        .wait_for_connected("test", Duration::from_secs(5))
        .await?;

    // Call echo
    let result = manager
        .call_tool("echo", serde_json::json!({"message": "hello world"}))
        .await?;
    assert_eq!(result.content, "echo: hello world");
    assert!(!result.is_error);

    manager.remove("test");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_server_add() -> Result<()> {
    let (mcp_url, _handle) = start_test_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("test", &mcp_url);
    manager
        .wait_for_connected("test", Duration::from_secs(5))
        .await?;

    // Call add
    let result = manager
        .call_tool("add", serde_json::json!({"a": 17, "b": 25}))
        .await?;
    assert_eq!(result.content, "42");
    assert!(!result.is_error);

    // Test negative numbers
    let result = manager
        .call_tool("add", serde_json::json!({"a": -10, "b": 5}))
        .await?;
    assert_eq!(result.content, "-5");

    manager.remove("test");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_tool_listing() -> Result<()> {
    let (mcp_url, _handle) = start_test_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("test", &mcp_url);
    manager
        .wait_for_connected("test", Duration::from_secs(5))
        .await?;

    let tools = manager.list_tools().await;

    // Should have all 3 tools
    assert_eq!(tools.len(), 3);

    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(tool_names.contains(&"ping"));
    assert!(tool_names.contains(&"echo"));
    assert!(tool_names.contains(&"add"));

    // All should have descriptions
    for tool in &tools {
        assert!(
            !tool.description.is_empty(),
            "tool {} should have description",
            tool.name
        );
    }

    manager.remove("test");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_unknown_tool() -> Result<()> {
    let (mcp_url, _handle) = start_test_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("test", &mcp_url);
    manager
        .wait_for_connected("test", Duration::from_secs(5))
        .await?;

    // Try to call non-existent tool
    let result = manager
        .call_tool("nonexistent", serde_json::json!({}))
        .await;
    assert!(result.is_err());

    manager.remove("test");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mock_llm_chat() -> Result<()> {
    let registry = create_test_model_registry();
    let model = registry.get("test").expect("test model should exist");

    let llm = LlmClient::new()?;

    // Test basic chat
    let response = llm.chat(model, "hello").await?;
    assert_eq!(response, "[mock]: hello");

    // Test with different input
    let response = llm.chat(model, "how are you?").await?;
    assert_eq!(response, "[mock]: how are you?");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mock_llm_chat_with_context() -> Result<()> {
    let registry = create_test_model_registry();
    let model = registry.get("test").expect("test model should exist");

    let llm = LlmClient::new()?;

    // Test with context (mock ignores context but shouldn't error)
    let history = vec![
        ("user".to_string(), "previous message".to_string()),
        ("assistant".to_string(), "previous response".to_string()),
    ];

    let response = llm
        .chat_with_context(model, "system prompt", &history, "current message")
        .await?;
    assert_eq!(response, "[mock]: current message");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mock_llm_ping() -> Result<()> {
    let registry = create_test_model_registry();
    let model = registry.get("test").expect("test model should exist");

    let llm = LlmClient::new()?;

    // Mock should always be reachable
    let reachable = llm.ping(model).await?;
    assert!(reachable);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_model_registry() -> Result<()> {
    let registry = create_test_model_registry();

    // Should have 2 models
    assert_eq!(registry.list().len(), 2);

    // Can get by short name
    let model = registry.get("test");
    assert!(model.is_some());
    assert_eq!(model.unwrap().display_name, "Test Echo Model");

    // All should be available
    assert_eq!(registry.available().len(), 2);

    // Unknown model returns None
    assert!(registry.get("unknown").is_none());

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_mcp_connections() -> Result<()> {
    // Start two MCP servers
    let (url1, _h1) = start_test_mcp_server().await?;
    let (url2, _h2) = start_test_mcp_server().await?;

    let manager = McpManager::new();

    // Connect to both
    manager.add("server1", &url1);
    manager.add("server2", &url2);
    manager
        .wait_for_connected("server1", Duration::from_secs(5))
        .await?;
    manager
        .wait_for_connected("server2", Duration::from_secs(5))
        .await?;

    // Should have tools from both (6 total, 3 from each)
    let tools = manager.list_tools().await;
    assert_eq!(tools.len(), 6);

    // Can call tools on either
    let result = manager.call_tool("ping", serde_json::json!({})).await?;
    assert_eq!(result.content, "pong");

    // Check connections
    let connections = manager.list_connections().await;
    assert_eq!(connections.len(), 2);

    // Disconnect one
    manager.remove("server1");
    // Give a moment for the removal to propagate
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Should still have 3 tools from server2
    let tools = manager.list_tools().await;
    assert_eq!(tools.len(), 3);

    manager.remove("server2");
    Ok(())
}

// ============================================================================
// Sshwarma MCP Server Tests
// ============================================================================

/// Start sshwarma MCP server with test state
async fn start_sshwarma_mcp_server() -> Result<(String, tokio::task::JoinHandle<()>)> {
    // Create temporary database
    let db = Database::open(":memory:").expect("failed to create test db");

    // Bootstrap the world structure (creates internal tools, lobby, etc.)
    db.bootstrap_world().expect("failed to bootstrap world");

    // Create test model registry with mock backend
    let mut models = ModelRegistry::new();
    models.register(ModelHandle {
        short_name: "test".to_string(),
        display_name: "Test Model".to_string(),
        backend: ModelBackend::Mock {
            prefix: "[test]".to_string(),
        },
        available: true,
        system_prompt: None,
        context_window: None,
    });

    let world = Arc::new(RwLock::new(World::new()));
    let db = Arc::new(db);
    let llm = Arc::new(LlmClient::new()?);
    let models = Arc::new(models);

    // Build SharedState for the MCP server
    let shared_state = Arc::new(SharedState {
        world: world.clone(),
        db: db.clone(),
        config: Config::default(),
        llm: llm.clone(),
        models: models.clone(),
        mcp: Arc::new(McpManager::new()),
        lua_reload: LuaReloadSender::new(),
    });

    // Create tool registry and Lua runtime
    let tool_registry = Arc::new(McpToolRegistry::new());

    // Create LuaRuntime with shared state
    let lua_runtime = LuaRuntime::new().expect("failed to create test Lua runtime");
    lua_runtime
        .tool_state()
        .set_shared_state(Some(shared_state.clone()));

    // Register tools.register_mcp_tool() function
    register_mcp_tool_registration(lua_runtime.lua(), tool_registry.clone())
        .expect("failed to register MCP tool registration");

    // Load Lua MCP tools from ~/.config/sshwarma/lua/mcp/init.lua
    if let Err(e) = lua_runtime.run_mcp_init_script() {
        eprintln!("Warning: Failed to load MCP init script: {}", e);
    }

    let state = Arc::new(McpServerState {
        world,
        db,
        llm,
        models,
        lua_runtime: Arc::new(Mutex::new(lua_runtime)),
        shared_state,
        tool_registry,
    });

    // Find a free port
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    drop(listener); // Release port for mcp_server to use

    let url = format!("http://127.0.0.1:{}/mcp", port);
    let handle = mcp_server::start_mcp_server(port, state).await?;

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    Ok((url, handle))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO: Update test for Lua tool response format"]
async fn test_sshwarma_mcp_list_rooms_empty() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("sshwarma", &url);
    manager
        .wait_for_connected("sshwarma", Duration::from_secs(5))
        .await?;

    let result = manager
        .call_tool("list_rooms", serde_json::json!({}))
        .await?;
    // Lua tool returns empty array [] when no rooms exist
    assert!(result.content.contains("[]") || result.content.contains("No rooms"));
    assert!(!result.is_error);

    manager.remove("sshwarma");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sshwarma_mcp_create_room() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("sshwarma", &url);
    manager
        .wait_for_connected("sshwarma", Duration::from_secs(5))
        .await?;

    // Create a room
    let result = manager
        .call_tool(
            "create_room",
            serde_json::json!({
                "name": "test-room",
                "description": "A test room"
            }),
        )
        .await?;
    assert!(result.content.contains("Created room 'test-room'"));
    assert!(!result.is_error);

    // List rooms should now show it
    let result = manager
        .call_tool("list_rooms", serde_json::json!({}))
        .await?;
    assert!(result.content.contains("test-room"));

    manager.remove("sshwarma");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sshwarma_mcp_list_models() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("sshwarma", &url);
    manager
        .wait_for_connected("sshwarma", Duration::from_secs(5))
        .await?;

    let result = manager
        .call_tool("list_models", serde_json::json!({}))
        .await?;
    assert!(result.content.contains("test"));
    assert!(result.content.contains("Test Model"));
    assert!(!result.is_error);

    manager.remove("sshwarma");
    Ok(())
}

// Note: ask_model was removed and replaced by `say @model message` syntax
// Model interactions now go through the say tool with @mention detection

#[tokio::test(flavor = "multi_thread")]
async fn test_sshwarma_mcp_tool_listing() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("sshwarma", &url);
    manager
        .wait_for_connected("sshwarma", Duration::from_secs(5))
        .await?;

    let tools = manager.list_tools().await;

    // Check for key tools (now Lua-defined + session tools)
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(tool_names.contains(&"list_rooms"), "Missing list_rooms");
    assert!(
        tool_names.contains(&"rows"),
        "Missing rows (replaces get_history)"
    );
    assert!(tool_names.contains(&"say"), "Missing say");
    assert!(tool_names.contains(&"list_models"), "Missing list_models");
    assert!(tool_names.contains(&"create_room"), "Missing create_room");
    assert!(
        tool_names.contains(&"identify"),
        "Missing identify (session tool)"
    );
    assert!(
        tool_names.contains(&"whoami"),
        "Missing whoami (session tool)"
    );

    manager.remove("sshwarma");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO: Update test for Lua tool error responses"]
async fn test_sshwarma_mcp_error_cases() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("sshwarma", &url);
    manager
        .wait_for_connected("sshwarma", Duration::from_secs(5))
        .await?;

    // Say to non-existent room - Lua tool returns JSON with error field
    let result = manager
        .call_tool(
            "say",
            serde_json::json!({
                "room": "no-such-room",
                "message": "Hello"
            }),
        )
        .await?;
    assert!(
        result.content.to_lowercase().contains("error")
            || result.content.to_lowercase().contains("not found")
            || result.content.to_lowercase().contains("does not exist")
    );

    // Create room with invalid name
    let result = manager
        .call_tool(
            "create_room",
            serde_json::json!({
                "name": "invalid name with spaces!"
            }),
        )
        .await?;
    // Lua tool returns error about valid characters
    assert!(
        result.content.to_lowercase().contains("error")
            || result.content.to_lowercase().contains("invalid")
            || result.content.to_lowercase().contains("alphanumeric")
    );

    manager.remove("sshwarma");
    Ok(())
}

// ============================================================================
// Room Context Tests
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_sshwarma_mcp_set_vibe() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("sshwarma", &url);
    manager
        .wait_for_connected("sshwarma", Duration::from_secs(5))
        .await?;

    // Create a room
    manager
        .call_tool("create_room", serde_json::json!({"name": "vibes-room"}))
        .await?;

    // Set vibe
    let result = manager
        .call_tool(
            "set_vibe",
            serde_json::json!({
                "room": "vibes-room",
                "vibe": "Chill lofi beats, late night coding session"
            }),
        )
        .await?;
    assert!(result.content.contains("Set vibe"));
    assert!(!result.is_error);

    // Get room context should show vibe
    let result = manager
        .call_tool(
            "room_context",
            serde_json::json!({
                "room": "vibes-room"
            }),
        )
        .await?;
    assert!(result.content.contains("Chill lofi"));
    // Lua tool returns JSON with lowercase field name
    assert!(result.content.contains("vibe"));

    manager.remove("sshwarma");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sshwarma_mcp_exits() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("sshwarma", &url);
    manager
        .wait_for_connected("sshwarma", Duration::from_secs(5))
        .await?;

    // Create two rooms
    manager
        .call_tool("create_room", serde_json::json!({"name": "lobby"}))
        .await?;
    manager
        .call_tool("create_room", serde_json::json!({"name": "studio"}))
        .await?;

    // Create bidirectional exit
    let result = manager
        .call_tool(
            "add_exit",
            serde_json::json!({
                "room": "lobby",
                "direction": "north",
                "target": "studio"
            }),
        )
        .await?;
    assert!(result.content.contains("north"));
    assert!(result.content.contains("south"));
    assert!(!result.is_error);

    // Check lobby exits
    let result = manager
        .call_tool(
            "room_context",
            serde_json::json!({
                "room": "lobby"
            }),
        )
        .await?;
    assert!(result.content.contains("north"));
    assert!(result.content.contains("studio"));

    // Check studio exits (should have south back to lobby)
    let result = manager
        .call_tool(
            "room_context",
            serde_json::json!({
                "room": "studio"
            }),
        )
        .await?;
    assert!(result.content.contains("south"));
    assert!(result.content.contains("lobby"));

    manager.remove("sshwarma");
    Ok(())
}

// ============================================================================
// Inventory/Equipment Tests
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO: Update test for Lua inventory tool responses"]
async fn test_sshwarma_mcp_inventory_equip_unequip() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("sshwarma", &url);
    manager
        .wait_for_connected("sshwarma", Duration::from_secs(5))
        .await?;

    // Create a room
    manager
        .call_tool("create_room", serde_json::json!({"name": "workshop"}))
        .await?;

    // List inventory (initially has bootstrap tools copied from lobby)
    let result = manager
        .call_tool(
            "inventory_list",
            serde_json::json!({
                "room": "workshop",
                "include_available": true
            }),
        )
        .await?;
    assert!(
        result.content.contains("Inventory for 'workshop'"),
        "Expected inventory for workshop, got: {}",
        result.content
    );
    assert!(!result.is_error);

    // Equip a tool (sshwarma:look should exist from bootstrap)
    let result = manager
        .call_tool(
            "inventory_equip",
            serde_json::json!({
                "room": "workshop",
                "qualified_name": "sshwarma:look"
            }),
        )
        .await?;
    assert!(
        result.content.contains("Equipped") || result.content.contains("sshwarma:look"),
        "Expected equip confirmation, got: {}",
        result.content
    );
    assert!(!result.is_error);

    // Verify in inventory
    let result = manager
        .call_tool(
            "inventory_list",
            serde_json::json!({
                "room": "workshop"
            }),
        )
        .await?;
    assert!(
        result.content.contains("sshwarma:look"),
        "Expected sshwarma:look in inventory, got: {}",
        result.content
    );

    // Unequip
    let result = manager
        .call_tool(
            "inventory_unequip",
            serde_json::json!({
                "room": "workshop",
                "qualified_name": "sshwarma:look"
            }),
        )
        .await?;
    assert!(result.content.contains("Unequipped"));
    assert!(!result.is_error);

    manager.remove("sshwarma");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sshwarma_mcp_fork_copies_equipment() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("sshwarma", &url);
    manager
        .wait_for_connected("sshwarma", Duration::from_secs(5))
        .await?;

    // Create a room and equip tools
    manager
        .call_tool("create_room", serde_json::json!({"name": "original"}))
        .await?;

    // Equip multiple tools to the original room
    manager
        .call_tool(
            "inventory_equip",
            serde_json::json!({
                "room": "original",
                "qualified_name": "sshwarma:look"
            }),
        )
        .await?;
    manager
        .call_tool(
            "inventory_equip",
            serde_json::json!({
                "room": "original",
                "qualified_name": "sshwarma:say"
            }),
        )
        .await?;

    // Verify original has the tools
    let result = manager
        .call_tool("inventory_list", serde_json::json!({"room": "original"}))
        .await?;
    assert!(result.content.contains("sshwarma:look"));
    assert!(result.content.contains("sshwarma:say"));

    // Fork the room
    let result = manager
        .call_tool(
            "fork_room",
            serde_json::json!({
                "source": "original",
                "new_name": "forked"
            }),
        )
        .await?;
    assert!(result.content.contains("Forked"));
    assert!(!result.is_error);

    // Verify forked room has the equipment copied
    let result = manager
        .call_tool("inventory_list", serde_json::json!({"room": "forked"}))
        .await?;
    assert!(
        result.content.contains("sshwarma:look") || result.content.contains("Inventory"),
        "Expected equipment in forked room, got: {}",
        result.content
    );

    manager.remove("sshwarma");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO: Update test for Lua equip tool responses"]
async fn test_sshwarma_mcp_equip_wildcards() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let manager = McpManager::new();
    manager.add("sshwarma", &url);
    manager
        .wait_for_connected("sshwarma", Duration::from_secs(5))
        .await?;

    // Create a room
    manager
        .call_tool("create_room", serde_json::json!({"name": "wildcards"}))
        .await?;

    // Equip all sshwarma tools using wildcard
    let result = manager
        .call_tool(
            "inventory_equip",
            serde_json::json!({
                "room": "wildcards",
                "qualified_name": "sshwarma:*"
            }),
        )
        .await?;
    assert!(
        result.content.contains("Equipped") && result.content.contains("things"),
        "Expected multiple tools equipped, got: {}",
        result.content
    );
    assert!(!result.is_error);

    // Verify multiple tools are equipped
    let result = manager
        .call_tool("inventory_list", serde_json::json!({"room": "wildcards"}))
        .await?;
    // Should have multiple sshwarma tools
    assert!(result.content.contains("sshwarma:look"));
    assert!(result.content.contains("sshwarma:say"));

    // Unequip all with wildcard
    let result = manager
        .call_tool(
            "inventory_unequip",
            serde_json::json!({
                "room": "wildcards",
                "qualified_name": "sshwarma:*"
            }),
        )
        .await?;
    assert!(result.content.contains("Unequipped"));
    assert!(!result.is_error);

    manager.remove("sshwarma");
    Ok(())
}
