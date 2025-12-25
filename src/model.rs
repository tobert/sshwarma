//! Model registry and handles

use std::collections::HashMap;

/// A model that can be addressed in a partyline
#[derive(Debug, Clone)]
pub struct ModelHandle {
    /// Short name for @mentions (e.g., "qwen-8b")
    pub short_name: String,
    /// Display name (e.g., "Qwen3-VL-8B-Instruct")
    pub display_name: String,
    /// Backend configuration
    pub backend: ModelBackend,
    /// Whether this model is currently available
    pub available: bool,
}

/// How to reach this model
#[derive(Debug, Clone)]
pub enum ModelBackend {
    /// Local llama.cpp via OpenAI-compatible API
    LlamaCpp {
        endpoint: String,
        model_name: String,
    },
    /// Claude API (future)
    Claude {
        model: String,
    },
    /// Gemini API (future)
    Gemini {
        model: String,
    },
    /// Ollama
    Ollama {
        endpoint: String,
        model: String,
    },
}

/// Registry of available models
pub struct ModelRegistry {
    models: HashMap<String, ModelHandle>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    /// Create a registry with default local models
    pub fn with_defaults(llm_endpoint: &str) -> Self {
        let mut registry = Self::new();

        // Qwen models on local llama.cpp
        registry.register(ModelHandle {
            short_name: "qwen-8b".to_string(),
            display_name: "Qwen3-VL-8B-Instruct".to_string(),
            backend: ModelBackend::LlamaCpp {
                endpoint: llm_endpoint.to_string(),
                model_name: "qwen3-vl-8b".to_string(),
            },
            available: true,
        });

        registry.register(ModelHandle {
            short_name: "qwen-4b".to_string(),
            display_name: "Qwen3-VL-4B-Instruct".to_string(),
            backend: ModelBackend::LlamaCpp {
                endpoint: llm_endpoint.to_string(),
                model_name: "qwen3-vl-4b".to_string(),
            },
            available: true,
        });

        registry
    }

    pub fn register(&mut self, model: ModelHandle) {
        self.models.insert(model.short_name.clone(), model);
    }

    pub fn get(&self, short_name: &str) -> Option<&ModelHandle> {
        self.models.get(short_name)
    }

    pub fn list(&self) -> Vec<&ModelHandle> {
        self.models.values().collect()
    }

    pub fn available(&self) -> Vec<&ModelHandle> {
        self.models.values().filter(|m| m.available).collect()
    }
}
