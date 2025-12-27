//! End-to-end integration tests for sshwarma
//!
//! Tests the MCP client and mock LLM without full SSH integration.
//! Full SSH e2e tests require more complex setup with key management.

use anyhow::Result;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpService,
        session::local::LocalSessionManager,
    },
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use sshwarma::db::Database;
use sshwarma::llm::LlmClient;
use sshwarma::mcp::McpClients;
use sshwarma::mcp_server::{self, McpServerState};
use sshwarma::model::{ModelBackend, ModelHandle, ModelRegistry};
use sshwarma::world::World;

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
    });

    registry.register(ModelHandle {
        short_name: "assistant".to_string(),
        display_name: "Test Assistant".to_string(),
        backend: ModelBackend::Mock {
            prefix: "I understand".to_string(),
        },
        available: true,
    });

    registry
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_mcp_server_ping() -> Result<()> {
    let (mcp_url, _handle) = start_test_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("test", &mcp_url).await?;

    // Verify ping tool exists
    let tools = clients.list_tools().await;
    assert!(tools.iter().any(|t| t.name == "ping"), "ping tool should exist");

    // Call ping
    let result = clients.call_tool("ping", serde_json::json!({})).await?;
    assert_eq!(result.content, "pong");
    assert!(!result.is_error);

    clients.disconnect("test").await?;
    Ok(())
}

#[tokio::test]
async fn test_mcp_server_echo() -> Result<()> {
    let (mcp_url, _handle) = start_test_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("test", &mcp_url).await?;

    // Call echo
    let result = clients.call_tool("echo", serde_json::json!({"message": "hello world"})).await?;
    assert_eq!(result.content, "echo: hello world");
    assert!(!result.is_error);

    clients.disconnect("test").await?;
    Ok(())
}

#[tokio::test]
async fn test_mcp_server_add() -> Result<()> {
    let (mcp_url, _handle) = start_test_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("test", &mcp_url).await?;

    // Call add
    let result = clients.call_tool("add", serde_json::json!({"a": 17, "b": 25})).await?;
    assert_eq!(result.content, "42");
    assert!(!result.is_error);

    // Test negative numbers
    let result = clients.call_tool("add", serde_json::json!({"a": -10, "b": 5})).await?;
    assert_eq!(result.content, "-5");

    clients.disconnect("test").await?;
    Ok(())
}

#[tokio::test]
async fn test_mcp_tool_listing() -> Result<()> {
    let (mcp_url, _handle) = start_test_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("test", &mcp_url).await?;

    let tools = clients.list_tools().await;

    // Should have all 3 tools
    assert_eq!(tools.len(), 3);

    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(tool_names.contains(&"ping"));
    assert!(tool_names.contains(&"echo"));
    assert!(tool_names.contains(&"add"));

    // All should have descriptions
    for tool in &tools {
        assert!(!tool.description.is_empty(), "tool {} should have description", tool.name);
    }

    clients.disconnect("test").await?;
    Ok(())
}

#[tokio::test]
async fn test_mcp_unknown_tool() -> Result<()> {
    let (mcp_url, _handle) = start_test_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("test", &mcp_url).await?;

    // Try to call non-existent tool
    let result = clients.call_tool("nonexistent", serde_json::json!({})).await;
    assert!(result.is_err());

    clients.disconnect("test").await?;
    Ok(())
}

#[tokio::test]
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

#[tokio::test]
async fn test_mock_llm_chat_with_context() -> Result<()> {
    let registry = create_test_model_registry();
    let model = registry.get("test").expect("test model should exist");

    let llm = LlmClient::new()?;

    // Test with context (mock ignores context but shouldn't error)
    let history = vec![
        ("user".to_string(), "previous message".to_string()),
        ("assistant".to_string(), "previous response".to_string()),
    ];

    let response = llm.chat_with_context(model, "system prompt", &history, "current message").await?;
    assert_eq!(response, "[mock]: current message");

    Ok(())
}

#[tokio::test]
async fn test_mock_llm_ping() -> Result<()> {
    let registry = create_test_model_registry();
    let model = registry.get("test").expect("test model should exist");

    let llm = LlmClient::new()?;

    // Mock should always be reachable
    let reachable = llm.ping(model).await?;
    assert!(reachable);

    Ok(())
}

#[tokio::test]
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

#[tokio::test]
async fn test_multiple_mcp_connections() -> Result<()> {
    // Start two MCP servers
    let (url1, _h1) = start_test_mcp_server().await?;
    let (url2, _h2) = start_test_mcp_server().await?;

    let clients = McpClients::new();

    // Connect to both
    clients.connect("server1", &url1).await?;
    clients.connect("server2", &url2).await?;

    // Should have tools from both (6 total, 3 from each)
    let tools = clients.list_tools().await;
    assert_eq!(tools.len(), 6);

    // Can call tools on either
    let result = clients.call_tool("ping", serde_json::json!({})).await?;
    assert_eq!(result.content, "pong");

    // Check connections
    let connections = clients.list_connections().await;
    assert_eq!(connections.len(), 2);

    // Disconnect one
    clients.disconnect("server1").await?;

    // Should still have 3 tools from server2
    let tools = clients.list_tools().await;
    assert_eq!(tools.len(), 3);

    clients.disconnect("server2").await?;
    Ok(())
}

// ============================================================================
// Sshwarma MCP Server Tests
// ============================================================================

/// Start sshwarma MCP server with test state
async fn start_sshwarma_mcp_server() -> Result<(String, tokio::task::JoinHandle<()>)> {
    // Create temporary database
    let db = Database::open(":memory:").expect("failed to create test db");

    // Create test model registry with mock backend
    let mut models = ModelRegistry::new();
    models.register(ModelHandle {
        short_name: "test".to_string(),
        display_name: "Test Model".to_string(),
        backend: ModelBackend::Mock {
            prefix: "[test]".to_string(),
        },
        available: true,
    });

    let state = Arc::new(McpServerState {
        world: Arc::new(tokio::sync::RwLock::new(World::new())),
        db: Arc::new(db),
        llm: Arc::new(LlmClient::new()?),
        models: Arc::new(models),
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

#[tokio::test]
async fn test_sshwarma_mcp_list_rooms_empty() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    let result = clients.call_tool("list_rooms", serde_json::json!({})).await?;
    assert!(result.content.contains("No rooms exist yet"));
    assert!(!result.is_error);

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_create_room() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Create a room
    let result = clients.call_tool("create_room", serde_json::json!({
        "name": "test-room",
        "description": "A test room"
    })).await?;
    assert!(result.content.contains("Created room 'test-room'"));
    assert!(!result.is_error);

    // List rooms should now show it
    let result = clients.call_tool("list_rooms", serde_json::json!({})).await?;
    assert!(result.content.contains("test-room"));

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_say_and_history() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Create a room first
    clients.call_tool("create_room", serde_json::json!({"name": "chat-room"})).await?;

    // Send a message
    let result = clients.call_tool("say", serde_json::json!({
        "room": "chat-room",
        "message": "Hello from Claude!",
        "sender": "claude"
    })).await?;
    assert!(result.content.contains("claude: Hello from Claude!"));
    assert!(!result.is_error);

    // Get history
    let result = clients.call_tool("get_history", serde_json::json!({
        "room": "chat-room",
        "limit": 10
    })).await?;
    assert!(result.content.contains("Hello from Claude!"));
    assert!(result.content.contains("claude"));

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_list_models() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    let result = clients.call_tool("list_models", serde_json::json!({})).await?;
    assert!(result.content.contains("test"));
    assert!(result.content.contains("Test Model"));
    assert!(!result.is_error);

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_ask_model() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Ask the mock model
    let result = clients.call_tool("ask_model", serde_json::json!({
        "model": "test",
        "message": "What is 2+2?"
    })).await?;
    // Mock model echoes with prefix
    assert!(result.content.contains("test:"));
    assert!(result.content.contains("What is 2+2?"));
    assert!(!result.is_error);

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_ask_model_with_room_context() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Create room and add some context
    clients.call_tool("create_room", serde_json::json!({"name": "context-room"})).await?;
    clients.call_tool("say", serde_json::json!({
        "room": "context-room",
        "message": "We're discussing math",
        "sender": "alice"
    })).await?;

    // Ask model with room context
    let result = clients.call_tool("ask_model", serde_json::json!({
        "model": "test",
        "message": "What were we discussing?",
        "room": "context-room"
    })).await?;
    // Model response should be recorded in room
    assert!(result.content.contains("test:"));
    assert!(!result.is_error);

    // History should now have the model's response
    let result = clients.call_tool("get_history", serde_json::json!({
        "room": "context-room"
    })).await?;
    assert!(result.content.contains("alice"));
    assert!(result.content.contains("test")); // model name in history

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_tool_listing() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    let tools = clients.list_tools().await;

    // Should have all 6 sshwarma tools
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(tool_names.contains(&"list_rooms"));
    assert!(tool_names.contains(&"get_history"));
    assert!(tool_names.contains(&"say"));
    assert!(tool_names.contains(&"ask_model"));
    assert!(tool_names.contains(&"list_models"));
    assert!(tool_names.contains(&"create_room"));

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_error_cases() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Say to non-existent room
    let result = clients.call_tool("say", serde_json::json!({
        "room": "no-such-room",
        "message": "Hello"
    })).await?;
    assert!(result.content.contains("does not exist"));

    // Get history from non-existent room
    let result = clients.call_tool("get_history", serde_json::json!({
        "room": "no-such-room"
    })).await?;
    assert!(result.content.contains("No messages"));

    // Ask unknown model
    let result = clients.call_tool("ask_model", serde_json::json!({
        "model": "unknown-model",
        "message": "Hello"
    })).await?;
    assert!(result.content.contains("Unknown model"));

    // Create room with invalid name
    let result = clients.call_tool("create_room", serde_json::json!({
        "name": "invalid name with spaces!"
    })).await?;
    assert!(result.content.contains("can only contain"));

    clients.disconnect("sshwarma").await?;
    Ok(())
}

// ============================================================================
// Room Context Tests
// ============================================================================

#[tokio::test]
async fn test_sshwarma_mcp_set_vibe() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Create a room
    clients.call_tool("create_room", serde_json::json!({"name": "vibes-room"})).await?;

    // Set vibe
    let result = clients.call_tool("set_vibe", serde_json::json!({
        "room": "vibes-room",
        "vibe": "Chill lofi beats, late night coding session"
    })).await?;
    assert!(result.content.contains("Set vibe"));
    assert!(!result.is_error);

    // Get room context should show vibe
    let result = clients.call_tool("room_context", serde_json::json!({
        "room": "vibes-room"
    })).await?;
    assert!(result.content.contains("Chill lofi"));
    assert!(result.content.contains("Vibe"));

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_journal() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Create a room
    clients.call_tool("create_room", serde_json::json!({"name": "journal-room"})).await?;

    // Add journal entries
    let result = clients.call_tool("journal_write", serde_json::json!({
        "room": "journal-room",
        "kind": "note",
        "content": "Started working on the beat"
    })).await?;
    assert!(result.content.contains("Added note"));
    assert!(!result.is_error);

    clients.call_tool("journal_write", serde_json::json!({
        "room": "journal-room",
        "kind": "decision",
        "content": "Using 120 BPM for the track"
    })).await?;

    clients.call_tool("journal_write", serde_json::json!({
        "room": "journal-room",
        "kind": "milestone",
        "content": "First draft complete!"
    })).await?;

    // Read all journal entries
    let result = clients.call_tool("journal_read", serde_json::json!({
        "room": "journal-room"
    })).await?;
    assert!(result.content.contains("Started working"));
    assert!(result.content.contains("120 BPM"));
    assert!(result.content.contains("First draft"));

    // Filter by kind
    let result = clients.call_tool("journal_read", serde_json::json!({
        "room": "journal-room",
        "kind": "decision"
    })).await?;
    assert!(result.content.contains("120 BPM"));
    assert!(!result.content.contains("Started working"));

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_asset_binding() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Create a room
    clients.call_tool("create_room", serde_json::json!({"name": "asset-room"})).await?;

    // Bind an asset
    let result = clients.call_tool("asset_bind", serde_json::json!({
        "room": "asset-room",
        "artifact_id": "abc123hash",
        "role": "drums",
        "notes": "808 kick pattern"
    })).await?;
    assert!(result.content.contains("Bound 'abc123hash'"));
    assert!(result.content.contains("drums"));
    assert!(!result.is_error);

    // Look up the asset
    let result = clients.call_tool("asset_lookup", serde_json::json!({
        "room": "asset-room",
        "role": "drums"
    })).await?;
    assert!(result.content.contains("abc123hash"));
    assert!(result.content.contains("808 kick pattern"));

    // Room context should show bound assets
    let result = clients.call_tool("room_context", serde_json::json!({
        "room": "asset-room"
    })).await?;
    assert!(result.content.contains("drums"));
    assert!(result.content.contains("abc123hash"));

    // Unbind the asset
    let result = clients.call_tool("asset_unbind", serde_json::json!({
        "room": "asset-room",
        "role": "drums"
    })).await?;
    assert!(result.content.contains("Unbound 'drums'"));

    // Asset should no longer be found
    let result = clients.call_tool("asset_lookup", serde_json::json!({
        "room": "asset-room",
        "role": "drums"
    })).await?;
    assert!(result.content.contains("No asset bound"));

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_exits() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Create two rooms
    clients.call_tool("create_room", serde_json::json!({"name": "lobby"})).await?;
    clients.call_tool("create_room", serde_json::json!({"name": "studio"})).await?;

    // Create bidirectional exit
    let result = clients.call_tool("add_exit", serde_json::json!({
        "room": "lobby",
        "direction": "north",
        "target": "studio"
    })).await?;
    assert!(result.content.contains("north"));
    assert!(result.content.contains("south"));
    assert!(!result.is_error);

    // Check lobby exits
    let result = clients.call_tool("room_context", serde_json::json!({
        "room": "lobby"
    })).await?;
    assert!(result.content.contains("north"));
    assert!(result.content.contains("studio"));

    // Check studio exits (should have south back to lobby)
    let result = clients.call_tool("room_context", serde_json::json!({
        "room": "studio"
    })).await?;
    assert!(result.content.contains("south"));
    assert!(result.content.contains("lobby"));

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_fork_room() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Create source room with context
    clients.call_tool("create_room", serde_json::json!({"name": "original"})).await?;
    clients.call_tool("set_vibe", serde_json::json!({
        "room": "original",
        "vibe": "Experimental ambient soundscape"
    })).await?;
    clients.call_tool("asset_bind", serde_json::json!({
        "room": "original",
        "artifact_id": "pad123",
        "role": "pad",
        "notes": "Main atmospheric pad"
    })).await?;

    // Fork the room
    let result = clients.call_tool("fork_room", serde_json::json!({
        "source": "original",
        "new_name": "variation-1"
    })).await?;
    assert!(result.content.contains("Forked 'variation-1'"));
    assert!(result.content.contains("Inherited"));
    assert!(!result.is_error);

    // Check forked room has inherited context
    let result = clients.call_tool("room_context", serde_json::json!({
        "room": "variation-1"
    })).await?;
    assert!(result.content.contains("Experimental ambient"));
    assert!(result.content.contains("pad"));
    assert!(result.content.contains("pad123"));
    assert!(result.content.contains("Forked from: original"));

    clients.disconnect("sshwarma").await?;
    Ok(())
}

#[tokio::test]
async fn test_sshwarma_mcp_room_context_full() -> Result<()> {
    let (url, _handle) = start_sshwarma_mcp_server().await?;

    let clients = McpClients::new();
    clients.connect("sshwarma", &url).await?;

    // Create a room with rich context
    clients.call_tool("create_room", serde_json::json!({"name": "rich-room"})).await?;

    // Add vibe
    clients.call_tool("set_vibe", serde_json::json!({
        "room": "rich-room",
        "vibe": "Deep house groove, 124 BPM"
    })).await?;

    // Add assets
    clients.call_tool("asset_bind", serde_json::json!({
        "room": "rich-room",
        "artifact_id": "kick001",
        "role": "kick"
    })).await?;
    clients.call_tool("asset_bind", serde_json::json!({
        "room": "rich-room",
        "artifact_id": "bass002",
        "role": "bassline"
    })).await?;

    // Add journal entries
    clients.call_tool("journal_write", serde_json::json!({
        "room": "rich-room",
        "kind": "note",
        "content": "Working on the groove"
    })).await?;
    clients.call_tool("journal_write", serde_json::json!({
        "room": "rich-room",
        "kind": "decision",
        "content": "Keep the bassline minimal"
    })).await?;

    // Get full context
    let result = clients.call_tool("room_context", serde_json::json!({
        "room": "rich-room"
    })).await?;

    // Should have all sections
    assert!(result.content.contains("# Room: rich-room"));
    assert!(result.content.contains("## Vibe"));
    assert!(result.content.contains("Deep house groove"));
    assert!(result.content.contains("## Bound Assets"));
    assert!(result.content.contains("kick"));
    assert!(result.content.contains("bassline"));
    assert!(result.content.contains("## Recent Journal"));
    assert!(result.content.contains("Working on the groove"));

    clients.disconnect("sshwarma").await?;
    Ok(())
}
