//! LLM integration via rig (multi-provider with native tool support)

use anyhow::Result;
use futures::StreamExt;
use opentelemetry::KeyValue;
use rig::agent::{MultiTurnStreamItem, Text};
use rig::client::{CompletionClient, Nothing, ProviderClient};
use rig::completion::{Chat, Message, Prompt};
use rig::providers::{anthropic, ollama, openai};
use rig::streaming::{StreamedAssistantContent, StreamedUserContent, StreamingPrompt};
use rig::tool::server::ToolServerHandle;
use rmcp::service::ServerSink;
use tokio::sync::mpsc;
use tracing::instrument;

use crate::model::{ModelBackend, ModelHandle};

/// Get or create the LLM request counter
fn llm_request_counter() -> opentelemetry::metrics::Counter<u64> {
    static COUNTER: std::sync::OnceLock<opentelemetry::metrics::Counter<u64>> =
        std::sync::OnceLock::new();
    COUNTER
        .get_or_init(|| {
            opentelemetry::global::meter("sshwarma")
                .u64_counter("sshwarma.llm.requests.total")
                .with_description("Total number of LLM requests")
                .build()
        })
        .clone()
}

/// Get or create the LLM latency histogram
fn llm_latency_histogram() -> opentelemetry::metrics::Histogram<f64> {
    static HISTOGRAM: std::sync::OnceLock<opentelemetry::metrics::Histogram<f64>> =
        std::sync::OnceLock::new();
    HISTOGRAM
        .get_or_init(|| {
            opentelemetry::global::meter("sshwarma")
                .f64_histogram("sshwarma.llm.duration_seconds")
                .with_description("LLM request duration in seconds")
                .with_unit("s")
                .build()
        })
        .clone()
}

use rmcp::model::Tool;

/// Convert history pairs (user, assistant) to rig Message format
fn history_to_messages(history: &[(String, String)]) -> Vec<Message> {
    history
        .iter()
        .flat_map(|(user_msg, assistant_msg)| {
            vec![
                Message::user(user_msg.clone()),
                Message::assistant(assistant_msg.clone()),
            ]
        })
        .collect()
}

/// Normalize a tool's input_schema for llama.cpp compatibility:
/// 1. Strip "default" keys (llama.cpp can't parse them)
/// 2. Add "type": "object" to schemas with only "description" (invalid JSON Schema)
///
/// This is needed because llama.cpp's JSON Schema parser is more restrictive
/// than the spec allows, and MCP tools often emit schemas with features it can't handle.
pub fn normalize_schema_for_llamacpp(tool: &Tool) -> Tool {
    fn normalize(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut cleaned: serde_json::Map<String, serde_json::Value> = map
                    .iter()
                    .filter(|(k, _)| k.as_str() != "default")
                    .map(|(k, v)| (k.clone(), normalize(v)))
                    .collect();

                // If this looks like a schema (has "description" but no "type"), add "type": "object"
                // This fixes schemars output for serde_json::Value fields
                if cleaned.contains_key("description") && !cleaned.contains_key("type") && !cleaned.contains_key("$ref") {
                    cleaned.insert("type".to_string(), serde_json::Value::String("object".to_string()));
                }

                serde_json::Value::Object(cleaned)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(normalize).collect())
            }
            other => other.clone(),
        }
    }

    let schema_value = serde_json::Value::Object(
        tool.input_schema.as_ref().clone().into_iter().collect()
    );
    let cleaned = normalize(&schema_value);
    let cleaned_map: serde_json::Map<String, serde_json::Value> = match cleaned {
        serde_json::Value::Object(m) => m,
        _ => tool.input_schema.as_ref().clone(),
    };

    Tool {
        name: tool.name.clone(),
        title: tool.title.clone(),
        description: tool.description.clone(),
        input_schema: std::sync::Arc::new(cleaned_map),
        output_schema: tool.output_schema.clone(),
        annotations: tool.annotations.clone(),
        icons: tool.icons.clone(),
        meta: tool.meta.clone(),
    }
}

/// Streaming response chunk
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Text content from the model (incremental)
    Text(String),
    /// Tool being called (name)
    ToolCall(String),
    /// Tool result summary
    ToolResult(String),
    /// Stream completed successfully
    Done,
    /// Error occurred during streaming
    Error(String),
}

/// Client for talking to LLMs via rig
///
/// Supports OpenAI, Anthropic, and Ollama with native tool calling.
///
/// ## Architecture Note
///
/// The methods in this client have repeated match blocks for each backend.
/// This is intentional—rig's provider clients have different concrete types
/// and don't implement a shared trait for agent building. LlamaCpp is
/// particularly different as it creates an OpenAI-compatible client on
/// each call. The duplication trades off against type safety and explicit
/// error handling per backend.
pub struct LlmClient {
    openai: Option<openai::Client>,
    anthropic: Option<anthropic::Client>,
    ollama: Option<ollama::Client>,
    ollama_endpoint: String,
}

impl LlmClient {
    /// Create a client with default configuration
    ///
    /// Uses environment variables for API keys:
    /// - OPENAI_API_KEY for OpenAI/GPT models
    /// - ANTHROPIC_API_KEY for Claude models
    /// - OLLAMA_API_BASE_URL for Ollama endpoint (default: http://localhost:11434)
    pub fn new() -> Result<Self> {
        // OpenAI client (optional - only if API key present)
        let openai = std::env::var("OPENAI_API_KEY").ok().map(|_| {
            openai::Client::from_env()
        });

        // Anthropic client (optional)
        let anthropic = std::env::var("ANTHROPIC_API_KEY").ok().map(|_| {
            anthropic::Client::from_env()
        });

        // Ollama client - default to localhost
        let ollama_endpoint = std::env::var("OLLAMA_API_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());

        let ollama = Some(
            ollama::Client::builder()
                .api_key(Nothing)
                .base_url(&ollama_endpoint)
                .build()
                .expect("failed to build ollama client")
        );

        Ok(Self {
            openai,
            anthropic,
            ollama,
            ollama_endpoint,
        })
    }

    /// Create a client with a custom Ollama endpoint
    pub fn with_ollama_endpoint(endpoint: &str) -> Result<Self> {
        let openai = std::env::var("OPENAI_API_KEY").ok().map(|_| {
            openai::Client::from_env()
        });

        let anthropic = std::env::var("ANTHROPIC_API_KEY").ok().map(|_| {
            anthropic::Client::from_env()
        });

        let ollama = Some(
            ollama::Client::builder()
                .api_key(Nothing)
                .base_url(endpoint)
                .build()
                .expect("failed to build ollama client")
        );

        Ok(Self {
            openai,
            anthropic,
            ollama,
            ollama_endpoint: endpoint.to_string(),
        })
    }

    /// Send a simple message to a model (no tools)
    pub async fn chat(&self, model: &ModelHandle, message: &str) -> Result<String> {
        self.chat_with_context(model, "", &[], message).await
    }

    /// Send a message with system prompt and conversation history
    ///
    /// History is a slice of (user_message, assistant_response) pairs that will be
    /// included as context for the model. The model will see these as prior conversation
    /// turns before the current message.
    #[instrument(
        name = "llm.chat",
        skip(self, system_prompt, history, message),
        fields(
            model.name = %model.short_name,
            model.backend = model.backend.variant_name(),
            history.turns = history.len(),
        )
    )]
    pub async fn chat_with_context(
        &self,
        model: &ModelHandle,
        system_prompt: &str,
        history: &[(String, String)],
        message: &str,
    ) -> Result<String> {
        // Handle mock backend for testing
        if let ModelBackend::Mock { prefix } = &model.backend {
            return Ok(format!("{}: {}", prefix, message));
        }

        // Convert history pairs to rig Message format
        let chat_history = history_to_messages(history);

        match &model.backend {
            ModelBackend::Ollama { model: model_id, .. } => {
                let client = self.ollama.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Ollama client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .build();

                let response = agent.chat(message, chat_history).await
                    .map_err(|e| anyhow::anyhow!("ollama error: {}", e))?;

                Ok(response)
            }

            ModelBackend::LlamaCpp { endpoint, model: model_id } => {
                // llama.cpp uses OpenAI-compatible API
                let client: openai::Client = openai::Client::builder()
                    .api_key("not-needed")
                    .base_url(endpoint)
                    .build()
                    .map_err(|e| anyhow::anyhow!("failed to create llamacpp client: {}", e))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .build();

                let response = agent.chat(message, chat_history).await
                    .map_err(|e| anyhow::anyhow!("llamacpp error: {}", e))?;

                Ok(response)
            }

            ModelBackend::OpenAI { model: model_id } => {
                let client = self.openai.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI client not configured - set OPENAI_API_KEY"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .build();

                let response = agent.chat(message, chat_history).await
                    .map_err(|e| anyhow::anyhow!("openai error: {}", e))?;

                Ok(response)
            }

            ModelBackend::Anthropic { model: model_id } => {
                let client = self.anthropic.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Anthropic client not configured - set ANTHROPIC_API_KEY"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .build();

                let response = agent.chat(message, chat_history).await
                    .map_err(|e| anyhow::anyhow!("anthropic error: {}", e))?;

                Ok(response)
            }

            ModelBackend::Gemini { .. } => {
                // TODO: Add Gemini support when rig adds it
                Err(anyhow::anyhow!("Gemini not yet supported with rig"))
            }

            ModelBackend::Mock { prefix } => {
                Ok(format!("{}: {}", prefix, message))
            }
        }
    }

    /// Chat with MCP tools and multi-turn execution
    ///
    /// This is the main entry point for tool-enabled LLM calls.
    /// Uses rig's native rmcp integration for tool handling.
    pub async fn chat_with_tools(
        &self,
        model: &ModelHandle,
        system_prompt: &str,
        message: &str,
        tools: Vec<rmcp::model::Tool>,
        mcp_peer: ServerSink,
        max_turns: usize,
    ) -> Result<String> {
        // Handle mock backend for testing
        if let ModelBackend::Mock { prefix } = &model.backend {
            return Ok(format!("{}: {}", prefix, message));
        }

        match &model.backend {
            ModelBackend::Ollama { model: model_id, .. } => {
                let client = self.ollama.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Ollama client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .rmcp_tools(tools, mcp_peer)
                    .build();

                let response = agent
                    .prompt(message)
                    .multi_turn(max_turns)
                    .await
                    .map_err(|e| anyhow::anyhow!("ollama error: {}", e))?;

                Ok(response)
            }

            ModelBackend::LlamaCpp { endpoint, model: model_id } => {
                // llama.cpp uses OpenAI Chat Completions API (not Responses API)
                let base_url = format!("{}/v1", endpoint);
                let client: openai::CompletionsClient = openai::CompletionsClient::builder()
                    .api_key("not-needed")
                    .base_url(&base_url)
                    .build()
                    .map_err(|e| anyhow::anyhow!("failed to create llamacpp client: {}", e))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .rmcp_tools(tools, mcp_peer)
                    .build();

                let response = agent
                    .prompt(message)
                    .multi_turn(max_turns)
                    .await
                    .map_err(|e| anyhow::anyhow!("llamacpp error: {}", e))?;

                Ok(response)
            }

            ModelBackend::OpenAI { model: model_id } => {
                let client = self.openai.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .rmcp_tools(tools, mcp_peer)
                    .build();

                let response = agent
                    .prompt(message)
                    .multi_turn(max_turns)
                    .await
                    .map_err(|e| anyhow::anyhow!("openai error: {}", e))?;

                Ok(response)
            }

            ModelBackend::Anthropic { model: model_id } => {
                let client = self.anthropic.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Anthropic client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .rmcp_tools(tools, mcp_peer)
                    .build();

                let response = agent
                    .prompt(message)
                    .multi_turn(max_turns)
                    .await
                    .map_err(|e| anyhow::anyhow!("anthropic error: {}", e))?;

                Ok(response)
            }

            ModelBackend::Gemini { .. } => {
                Err(anyhow::anyhow!("Gemini not yet supported with rig"))
            }

            ModelBackend::Mock { prefix } => {
                Ok(format!("{}: {}", prefix, message))
            }
        }
    }

    /// Chat with a ToolServer handle (supports both internal and MCP tools)
    ///
    /// This is the main entry point when you have combined tools.
    /// Build a ToolServer with internal sshwarma tools + optional MCP tools,
    /// then pass the handle here.
    #[instrument(
        name = "llm.chat_with_tools",
        skip(self, system_prompt, message, tool_server_handle),
        fields(
            model.name = %model.short_name,
            model.backend = model.backend.variant_name(),
            max_turns = max_turns,
        )
    )]
    pub async fn chat_with_tool_server(
        &self,
        model: &ModelHandle,
        system_prompt: &str,
        message: &str,
        tool_server_handle: ToolServerHandle,
        max_turns: usize,
    ) -> Result<String> {
        // Handle mock backend for testing
        if let ModelBackend::Mock { prefix } = &model.backend {
            return Ok(format!("{}: {}", prefix, message));
        }

        match &model.backend {
            ModelBackend::Ollama { model: model_id, .. } => {
                let client = self.ollama.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Ollama client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .tool_server_handle(tool_server_handle)
                    .build();

                agent
                    .prompt(message)
                    .multi_turn(max_turns)
                    .await
                    .map_err(|e| anyhow::anyhow!("ollama error: {}", e))
            }

            ModelBackend::LlamaCpp { endpoint, model: model_id } => {
                // llama.cpp uses OpenAI Chat Completions API (not Responses API)
                let base_url = format!("{}/v1", endpoint);
                let client: openai::CompletionsClient = openai::CompletionsClient::builder()
                    .api_key("not-needed")
                    .base_url(&base_url)
                    .build()
                    .map_err(|e| anyhow::anyhow!("failed to create llamacpp client: {}", e))?;

                // Debug: log available tools
                match tool_server_handle.get_tool_defs(None).await {
                    Ok(defs) => {
                        let names: Vec<_> = defs.iter().map(|d| d.name.as_str()).collect();
                        tracing::info!("llamacpp agent has {} tools: {:?}", defs.len(), names);
                    }
                    Err(e) => tracing::warn!("failed to get tool defs for logging: {}", e),
                }

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .tool_server_handle(tool_server_handle)
                    .build();

                agent
                    .prompt(message)
                    .multi_turn(max_turns)
                    .await
                    .map_err(|e| anyhow::anyhow!("llamacpp error: {}", e))
            }

            ModelBackend::OpenAI { model: model_id } => {
                let client = self.openai.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .tool_server_handle(tool_server_handle)
                    .build();

                agent
                    .prompt(message)
                    .multi_turn(max_turns)
                    .await
                    .map_err(|e| anyhow::anyhow!("openai error: {}", e))
            }

            ModelBackend::Anthropic { model: model_id } => {
                let client = self.anthropic.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Anthropic client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .tool_server_handle(tool_server_handle)
                    .build();

                agent
                    .prompt(message)
                    .multi_turn(max_turns)
                    .await
                    .map_err(|e| anyhow::anyhow!("anthropic error: {}", e))
            }

            ModelBackend::Gemini { .. } => {
                Err(anyhow::anyhow!("Gemini not yet supported with rig"))
            }

            ModelBackend::Mock { prefix } => {
                Ok(format!("{}: {}", prefix, message))
            }
        }
    }

    /// Stream a chat with tools, sending chunks as they arrive
    ///
    /// This is the streaming version of `chat_with_tool_server`.
    /// Sends StreamChunk items through the channel as they arrive.
    #[instrument(
        name = "llm.stream_with_tools",
        skip(self, system_prompt, message, tool_server_handle, tx),
        fields(
            model.name = %model.short_name,
            model.backend = model.backend.variant_name(),
            max_turns = max_turns,
        )
    )]
    pub async fn stream_with_tool_server(
        &self,
        model: &ModelHandle,
        system_prompt: &str,
        message: &str,
        tool_server_handle: ToolServerHandle,
        tx: mpsc::Sender<StreamChunk>,
        max_turns: usize,
    ) -> Result<()> {
        let start = std::time::Instant::now();
        let attrs = [
            KeyValue::new("model", model.short_name.clone()),
            KeyValue::new("backend", model.backend.variant_name()),
        ];

        // Record request start
        llm_request_counter().add(1, &attrs);

        // Macro to process streaming items - avoids code duplication across providers
        macro_rules! process_stream {
            ($stream:expr, $tx:expr, $start:expr, $attrs:expr) => {{
                while let Some(item) = $stream.next().await {
                    match item {
                        Ok(MultiTurnStreamItem::StreamAssistantItem(content)) => {
                            match content {
                                StreamedAssistantContent::Text(Text { text }) => {
                                    let _ = $tx.send(StreamChunk::Text(text)).await;
                                }
                                StreamedAssistantContent::ToolCall(tool_call) => {
                                    let _ = $tx.send(StreamChunk::ToolCall(
                                        tool_call.function.name.clone()
                                    )).await;
                                }
                                StreamedAssistantContent::ToolCallDelta { .. } => {}
                                StreamedAssistantContent::Reasoning(_) => {}
                                StreamedAssistantContent::ReasoningDelta { .. } => {}
                                StreamedAssistantContent::Final(_) => {}
                            }
                        }
                        Ok(MultiTurnStreamItem::StreamUserItem(
                            StreamedUserContent::ToolResult(result)
                        )) => {
                            let summary = format!("{}: done", result.id);
                            let _ = $tx.send(StreamChunk::ToolResult(summary)).await;
                        }
                        Ok(MultiTurnStreamItem::FinalResponse(_)) => {}
                        Ok(_) => {} // Handle future non-exhaustive variants
                        Err(e) => {
                            llm_latency_histogram().record($start.elapsed().as_secs_f64(), $attrs);
                            let _ = $tx.send(StreamChunk::Error(e.to_string())).await;
                            return Ok(());
                        }
                    }
                }
                llm_latency_histogram().record($start.elapsed().as_secs_f64(), $attrs);
                let _ = $tx.send(StreamChunk::Done).await;
                Ok(())
            }};
        }

        // Handle mock backend for testing
        if let ModelBackend::Mock { prefix } = &model.backend {
            let response = format!("{}: {}", prefix, message);
            let _ = tx.send(StreamChunk::Text(response)).await;
            let _ = tx.send(StreamChunk::Done).await;
            llm_latency_histogram().record(start.elapsed().as_secs_f64(), &attrs);
            return Ok(());
        }

        match &model.backend {
            ModelBackend::Ollama { model: model_id, .. } => {
                let client = self.ollama.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Ollama client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .tool_server_handle(tool_server_handle)
                    .build();

                let mut stream = agent
                    .stream_prompt(message)
                    .multi_turn(max_turns)
                    .await;

                process_stream!(stream, tx, start, &attrs)
            }

            ModelBackend::LlamaCpp { endpoint, model: model_id } => {
                // llama.cpp uses OpenAI Chat Completions API (not Responses API)
                let base_url = format!("{}/v1", endpoint);
                let client: openai::CompletionsClient = openai::CompletionsClient::builder()
                    .api_key("not-needed")
                    .base_url(&base_url)
                    .build()
                    .map_err(|e| anyhow::anyhow!("failed to create llamacpp client: {}", e))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .tool_server_handle(tool_server_handle)
                    .build();

                let mut stream = agent
                    .stream_prompt(message)
                    .multi_turn(max_turns)
                    .await;

                process_stream!(stream, tx, start, &attrs)
            }

            ModelBackend::OpenAI { model: model_id } => {
                let client = self.openai.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .tool_server_handle(tool_server_handle)
                    .build();

                let mut stream = agent
                    .stream_prompt(message)
                    .multi_turn(max_turns)
                    .await;

                process_stream!(stream, tx, start, &attrs)
            }

            ModelBackend::Anthropic { model: model_id } => {
                let client = self.anthropic.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Anthropic client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .tool_server_handle(tool_server_handle)
                    .build();

                let mut stream = agent
                    .stream_prompt(message)
                    .multi_turn(max_turns)
                    .await;

                process_stream!(stream, tx, start, &attrs)
            }

            ModelBackend::Gemini { .. } => {
                let _ = tx.send(StreamChunk::Error("Gemini not yet supported".to_string())).await;
                Ok(())
            }

            ModelBackend::Mock { prefix } => {
                let response = format!("{}: {}", prefix, message);
                let _ = tx.send(StreamChunk::Text(response)).await;
                let _ = tx.send(StreamChunk::Done).await;
                Ok(())
            }
        }
    }


    /// Stream a chat response (no tools)
    ///
    /// Sends text chunks as they arrive from the model.
    /// For tool-enabled streaming, use `stream_with_tool_server` instead.
    pub async fn chat_stream(
        &self,
        model: &ModelHandle,
        system_prompt: &str,
        history: &[(String, String)],
        message: &str,
        tx: mpsc::Sender<StreamChunk>,
    ) -> Result<()> {
        // Macro to process streaming items - same pattern as stream_with_tool_server
        macro_rules! process_simple_stream {
            ($stream:expr, $tx:expr) => {{
                while let Some(item) = $stream.next().await {
                    match item {
                        Ok(MultiTurnStreamItem::StreamAssistantItem(content)) => {
                            match content {
                                StreamedAssistantContent::Text(Text { text }) => {
                                    let _ = $tx.send(StreamChunk::Text(text)).await;
                                }
                                // No tool calls expected in simple streaming
                                StreamedAssistantContent::ToolCall(_) => {}
                                StreamedAssistantContent::ToolCallDelta { .. } => {}
                                StreamedAssistantContent::Reasoning(_) => {}
                                StreamedAssistantContent::ReasoningDelta { .. } => {}
                                StreamedAssistantContent::Final(_) => {}
                            }
                        }
                        Ok(MultiTurnStreamItem::FinalResponse(_)) => {}
                        Ok(_) => {} // Handle future non-exhaustive variants
                        Err(e) => {
                            let _ = $tx.send(StreamChunk::Error(e.to_string())).await;
                            return Ok(());
                        }
                    }
                }
                let _ = $tx.send(StreamChunk::Done).await;
                Ok(())
            }};
        }

        // Handle mock backend for testing
        if let ModelBackend::Mock { prefix } = &model.backend {
            let response = format!("{}: {}", prefix, message);
            let _ = tx.send(StreamChunk::Text(response)).await;
            let _ = tx.send(StreamChunk::Done).await;
            return Ok(());
        }

        // Convert history pairs to rig Message format
        let chat_history = history_to_messages(history);

        match &model.backend {
            ModelBackend::Ollama { model: model_id, .. } => {
                let client = self.ollama.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Ollama client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .build();

                let mut stream = agent
                    .stream_prompt(message)
                    .with_history(chat_history)
                    .await;

                process_simple_stream!(stream, tx)
            }

            ModelBackend::LlamaCpp { endpoint, model: model_id } => {
                // llama.cpp uses OpenAI-compatible API
                let client: openai::Client = openai::Client::builder()
                    .api_key("not-needed")
                    .base_url(endpoint)
                    .build()
                    .map_err(|e| anyhow::anyhow!("failed to create llamacpp client: {}", e))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .build();

                let mut stream = agent
                    .stream_prompt(message)
                    .with_history(chat_history)
                    .await;

                process_simple_stream!(stream, tx)
            }

            ModelBackend::OpenAI { model: model_id } => {
                let client = self.openai.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .build();

                let mut stream = agent
                    .stream_prompt(message)
                    .with_history(chat_history)
                    .await;

                process_simple_stream!(stream, tx)
            }

            ModelBackend::Anthropic { model: model_id } => {
                let client = self.anthropic.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Anthropic client not configured"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .build();

                let mut stream = agent
                    .stream_prompt(message)
                    .with_history(chat_history)
                    .await;

                process_simple_stream!(stream, tx)
            }

            ModelBackend::Gemini { .. } => {
                let _ = tx.send(StreamChunk::Error("Gemini not yet supported".to_string())).await;
                Ok(())
            }

            ModelBackend::Mock { prefix } => {
                let response = format!("{}: {}", prefix, message);
                let _ = tx.send(StreamChunk::Text(response)).await;
                let _ = tx.send(StreamChunk::Done).await;
                Ok(())
            }
        }
    }

    /// Generate flavor text for room descriptions
    pub async fn generate_flavor(&self, model: &ModelHandle, context: &FlavorContext) -> Result<String> {
        let prompt = format!(
            "Generate a brief, evocative 2-3 sentence description of this collaborative space. \
             Be creative but grounded. No fantasy elements.\n\n\
             Room: {}\n\
             Users present: {}\n\
             Models available: {}\n\
             Recent activity: {}\n\
             Artifacts: {} items\n\n\
             Description:",
            context.room_name,
            context.users.join(", "),
            context.models.join(", "),
            context.recent_activity,
            context.artifact_count
        );

        self.chat(model, &prompt).await
    }

    /// Check if a model is reachable
    pub async fn ping(&self, model: &ModelHandle) -> Result<bool> {
        // Mock backend is always reachable
        if matches!(&model.backend, ModelBackend::Mock { .. }) {
            return Ok(true);
        }

        // Try a simple prompt
        match self.chat(model, "ping").await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Get the configured Ollama endpoint
    pub fn ollama_endpoint(&self) -> &str {
        &self.ollama_endpoint
    }
}

impl Default for LlmClient {
    fn default() -> Self {
        Self::new().expect("failed to create LlmClient")
    }
}

/// Context for flavor text generation
pub struct FlavorContext {
    pub room_name: String,
    pub users: Vec<String>,
    pub models: Vec<String>,
    pub recent_activity: String,
    pub artifact_count: usize,
}
