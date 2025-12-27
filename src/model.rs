//! Model registry and handles

use std::collections::HashMap;

use crate::config::{ModelConfig, ModelsConfig};

/// A model that can be addressed in a room
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
    /// Ollama/llama.cpp via OpenAI-compatible API
    Ollama {
        endpoint: String,
        model: String,
    },
    /// OpenAI API
    OpenAI {
        model: String,
    },
    /// Anthropic Claude API
    Anthropic {
        model: String,
    },
    /// Google Gemini API
    Gemini {
        model: String,
    },
    /// Mock backend for testing - echoes input with prefix
    Mock {
        prefix: String,
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

    /// Load models from configuration
    pub fn from_config(config: &ModelsConfig) -> Self {
        let mut registry = Self::new();

        for model_config in &config.models {
            if !model_config.enabled {
                tracing::debug!("skipping disabled model: {}", model_config.name);
                continue;
            }

            if let Some(handle) = Self::model_from_config(model_config, &config.ollama_endpoint) {
                tracing::info!(
                    "registered model @{} ({}) via {}",
                    handle.short_name,
                    handle.display_name,
                    model_config.backend
                );
                registry.register(handle);
            }
        }

        registry
    }

    /// Convert a ModelConfig to a ModelHandle
    fn model_from_config(config: &ModelConfig, default_ollama_endpoint: &str) -> Option<ModelHandle> {
        let backend = match config.backend.as_str() {
            "ollama" | "llamacpp" | "llama.cpp" => {
                let endpoint = config
                    .endpoint
                    .clone()
                    .unwrap_or_else(|| default_ollama_endpoint.to_string());
                ModelBackend::Ollama {
                    endpoint,
                    model: config.model.clone(),
                }
            }
            "openai" => ModelBackend::OpenAI {
                model: config.model.clone(),
            },
            "anthropic" | "claude" => ModelBackend::Anthropic {
                model: config.model.clone(),
            },
            "gemini" | "google" => ModelBackend::Gemini {
                model: config.model.clone(),
            },
            unknown => {
                tracing::warn!("unknown backend '{}' for model {}", unknown, config.name);
                return None;
            }
        };

        Some(ModelHandle {
            short_name: config.name.clone(),
            display_name: config.display.clone(),
            backend,
            available: true,
        })
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

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
