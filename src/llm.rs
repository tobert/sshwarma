//! LLM integration via genai (multi-provider)

use anyhow::Result;
use futures::StreamExt;
use genai::chat::{ChatMessage, ChatRequest, ChatStreamEvent};
use genai::Client;
use tokio::sync::mpsc;

use crate::model::{ModelBackend, ModelHandle};

/// Streaming response chunk
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Text content from the model
    Text(String),
    /// Stream completed successfully
    Done,
    /// Error occurred during streaming
    Error(String),
}

/// Client for talking to LLMs via genai
pub struct LlmClient {
    client: Client,
}

impl LlmClient {
    /// Create a client with default configuration
    /// Uses environment variables for API keys:
    /// - OPENAI_API_KEY for OpenAI/GPT models
    /// - ANTHROPIC_API_KEY for Claude models
    /// - GEMINI_API_KEY for Gemini models
    /// - Ollama models don't need a key
    pub fn new() -> Result<Self> {
        let client = Client::default();
        Ok(Self { client })
    }

    /// Create a client with a custom auth resolver for local llama.cpp
    pub fn with_ollama_endpoint(endpoint: &str) -> Result<Self> {
        // For local llama.cpp via Ollama-compatible API, set OLLAMA_HOST
        std::env::set_var("OLLAMA_HOST", endpoint);
        let client = Client::default();
        Ok(Self { client })
    }

    /// Get the model identifier string for genai based on backend
    fn model_id(model: &ModelHandle) -> Option<String> {
        match &model.backend {
            ModelBackend::Ollama { model, .. } => Some(model.clone()),
            ModelBackend::OpenAI { model } => Some(model.clone()),
            ModelBackend::Anthropic { model } => Some(model.clone()),
            ModelBackend::Gemini { model } => Some(model.clone()),
            ModelBackend::Mock { .. } => None, // Mock doesn't use genai
        }
    }

    /// Send a message to a model and get a response
    pub async fn chat(&self, model: &ModelHandle, message: &str) -> Result<String> {
        // Handle mock backend for testing
        if let ModelBackend::Mock { prefix } = &model.backend {
            return Ok(format!("{}: {}", prefix, message));
        }

        let model_id = Self::model_id(model).expect("non-mock model should have id");

        let chat_req = ChatRequest::new(vec![
            ChatMessage::user(message),
        ]);

        let response = self.client.exec_chat(&model_id, chat_req, None).await?;
        let content = response
            .into_first_text()
            .unwrap_or_else(|| "[no response]".to_string());

        Ok(content)
    }

    /// Send a message with system prompt and conversation history
    pub async fn chat_with_context(
        &self,
        model: &ModelHandle,
        system_prompt: &str,
        history: &[(String, String)], // (role, content) pairs
        message: &str,
    ) -> Result<String> {
        // Handle mock backend for testing
        if let ModelBackend::Mock { prefix } = &model.backend {
            return Ok(format!("{}: {}", prefix, message));
        }

        let model_id = Self::model_id(model).expect("non-mock model should have id");

        let mut messages = vec![ChatMessage::system(system_prompt)];

        // Add conversation history
        for (role, content) in history {
            match role.as_str() {
                "user" => messages.push(ChatMessage::user(content)),
                "assistant" => messages.push(ChatMessage::assistant(content)),
                _ => {} // Skip unknown roles
            }
        }

        // Add the current message
        messages.push(ChatMessage::user(message));

        let chat_req = ChatRequest::new(messages);
        let response = self.client.exec_chat(&model_id, chat_req, None).await?;
        let content = response
            .into_first_text()
            .unwrap_or_else(|| "[no response]".to_string());

        Ok(content)
    }

    /// Stream a chat response, sending chunks through the channel as they arrive
    pub async fn chat_stream(
        &self,
        model: &ModelHandle,
        system_prompt: &str,
        history: &[(String, String)],
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

        let model_id = Self::model_id(model).expect("non-mock model should have id");

        let mut messages = vec![ChatMessage::system(system_prompt)];

        for (role, content) in history {
            match role.as_str() {
                "user" => messages.push(ChatMessage::user(content)),
                "assistant" => messages.push(ChatMessage::assistant(content)),
                _ => {}
            }
        }

        messages.push(ChatMessage::user(message));

        let chat_req = ChatRequest::new(messages);
        let chat_stream = self.client.exec_chat_stream(&model_id, chat_req, None).await?;

        let mut stream = chat_stream.stream;
        while let Some(result) = stream.next().await {
            match result {
                Ok(event) => match event {
                    ChatStreamEvent::Chunk(chunk) => {
                        if tx.send(StreamChunk::Text(chunk.content)).await.is_err() {
                            break; // Receiver dropped
                        }
                    }
                    ChatStreamEvent::End(_) => {
                        let _ = tx.send(StreamChunk::Done).await;
                        break;
                    }
                    _ => {} // Ignore other events for now
                },
                Err(e) => {
                    let _ = tx.send(StreamChunk::Error(e.to_string())).await;
                    break;
                }
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

        let model_id = Self::model_id(model).expect("non-mock model should have id");
        let chat_req = ChatRequest::new(vec![
            ChatMessage::user("ping"),
        ]);

        match self.client.exec_chat(&model_id, chat_req, None).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
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
