//! Abstraction des fournisseurs de LLM : un trait unique, une implémentation par API.

mod fallback;
mod openai_compat;

pub use fallback::FallbackProvider;
pub use openai_compat::{OpenAiCompatEmbedder, OpenAiCompatProvider};

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }
}

/// Réglages d'une réponse non streamée (tâches internes : extraction, résumés…).
#[derive(Debug, Clone, Default)]
pub struct CompletionOptions {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Force une sortie JSON valide (`response_format: json_object`).
    pub json: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("requête HTTP échouée : {0}")]
    Http(#[from] reqwest::Error),
    #[error("le fournisseur a répondu {status} : {body}")]
    Status {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("flux SSE invalide : {0}")]
    Stream(String),
    #[error("JSON invalide : {0}")]
    Json(#[from] serde_json::Error),
}

/// Flux des fragments de texte générés, dans l'ordre.
pub type TokenStream = BoxStream<'static, Result<String, LlmError>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat_stream(&self, messages: Vec<Message>) -> Result<TokenStream, LlmError>;

    async fn complete(
        &self,
        messages: Vec<Message>,
        options: CompletionOptions,
    ) -> Result<String, LlmError>;
}
