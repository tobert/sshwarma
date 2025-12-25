//! LLM integration via OpenAI-compatible API (llama.cpp)

use anyhow::Result;
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs,
    },
    Client,
};

use crate::model::{ModelBackend, ModelHandle};

/// Client for talking to LLMs
pub struct LlmClient {
    client: Client<OpenAIConfig>,
}

impl LlmClient {
    /// Create a client pointing at llama.cpp
    pub fn new(endpoint: &str) -> Self {
        let config = OpenAIConfig::new()
            .with_api_base(endpoint)
            .with_api_key("not-needed"); // llama.cpp doesn't need a key

        Self {
            client: Client::with_config(config),
        }
    }

    /// Send a message to a model and get a response
    pub async fn chat(&self, model: &ModelHandle, message: &str) -> Result<String> {
        let model_name = match &model.backend {
            ModelBackend::LlamaCpp { model_name, .. } => model_name.clone(),
            ModelBackend::Ollama { model, .. } => model.clone(),
            _ => return Err(anyhow::anyhow!("unsupported backend for chat")),
        };

        let request = CreateChatCompletionRequestArgs::default()
            .model(&model_name)
            .messages(vec![ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(message)
                    .build()?,
            )])
            .build()?;

        let response = self.client.chat().create(request).await?;

        let content = response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_else(|| "[no response]".to_string());

        Ok(content)
    }

    /// Generate flavor text for room descriptions
    pub async fn generate_flavor(&self, model: &ModelHandle, context: &RoomContext) -> Result<String> {
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
}

/// Context for flavor text generation
pub struct RoomContext {
    pub room_name: String,
    pub users: Vec<String>,
    pub models: Vec<String>,
    pub recent_activity: String,
    pub artifact_count: usize,
}
