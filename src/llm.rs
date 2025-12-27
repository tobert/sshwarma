//! LLM integration via rig (multi-provider with native tool support)

use anyhow::Result;
use futures::StreamExt;
use rig::agent::{MultiTurnStreamItem, Text};
use rig::client::{CompletionClient, Nothing, ProviderClient};
use rig::completion::Prompt;
use rig::providers::{anthropic, ollama, openai};
use rig::streaming::{StreamedAssistantContent, StreamedUserContent, StreamingPrompt};
use rig::tool::server::ToolServerHandle;
use rmcp::service::ServerSink;
use tokio::sync::mpsc;

use crate::model::{ModelBackend, ModelHandle};

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
    pub async fn chat_with_context(
        &self,
        model: &ModelHandle,
        system_prompt: &str,
        _history: &[(String, String)], // TODO: integrate history
        message: &str,
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
                    .build();

                let response = agent.prompt(message).await
                    .map_err(|e| anyhow::anyhow!("ollama error: {}", e))?;

                Ok(response)
            }

            ModelBackend::OpenAI { model: model_id } => {
                let client = self.openai.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI client not configured - set OPENAI_API_KEY"))?;

                let agent = client
                    .agent(model_id)
                    .preamble(system_prompt)
                    .build();

                let response = agent.prompt(message).await
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

                let response = agent.prompt(message).await
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
    pub async fn stream_with_tool_server(
        &self,
        model: &ModelHandle,
        system_prompt: &str,
        message: &str,
        tool_server_handle: ToolServerHandle,
        tx: mpsc::Sender<StreamChunk>,
        max_turns: usize,
    ) -> Result<()> {
        // Macro to process streaming items - avoids code duplication across providers
        macro_rules! process_stream {
            ($stream:expr, $tx:expr) => {{
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

                process_stream!(stream, tx)
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

                process_stream!(stream, tx)
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

                process_stream!(stream, tx)
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
    /// For tool-enabled streaming, use the agent directly with stream_prompt.
    pub async fn chat_stream(
        &self,
        model: &ModelHandle,
        system_prompt: &str,
        _history: &[(String, String)],
        message: &str,
        tx: mpsc::Sender<StreamChunk>,
    ) -> Result<()> {
        // Handle mock backend for testing
        if let ModelBackend::Mock { prefix } = &model.backend {
            let response = format!("{}: {}", prefix, message);
            let _ = tx.send(StreamChunk::Text(response)).await;
            let _ = tx.send(StreamChunk::Done).await;
            return Ok(());
        }

        // For now, fall back to non-streaming and send result all at once
        // TODO: implement proper streaming with rig's stream_prompt
        match self.chat_with_context(model, system_prompt, &[], message).await {
            Ok(response) => {
                let _ = tx.send(StreamChunk::Text(response)).await;
                let _ = tx.send(StreamChunk::Done).await;
            }
            Err(e) => {
                let _ = tx.send(StreamChunk::Error(e.to_string())).await;
            }
        }

        Ok(())
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
