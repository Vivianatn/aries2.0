//! Abstraction des fournisseurs de LLM : un trait unique, une implémentation par API.

mod fallback;
mod openai_compat;
mod quota;
mod tool_calls;

pub use fallback::FallbackProvider;
pub use openai_compat::{OpenAiCompatEmbedder, OpenAiCompatProvider};
pub use quota::DailyQuota;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Outils demandés par l'assistant (rôle `assistant`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Appel auquel répond ce message (rôle `tool`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant_tool_calls(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            tool_calls,
            ..Self::new(Role::Assistant, content)
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tool_call_id: Some(tool_call_id.into()),
            ..Self::new(Role::Tool, content)
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }
}

/// Outil proposé au modèle : nom, description et schéma JSON des arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Appel d'outil décidé par le modèle (format OpenAI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: ToolCallKind,
    pub function: FunctionCall,
    /// Données propres au fournisseur (ex. signatures de réflexion Gemini), renvoyées telles quelles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_content: Option<Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolCallKind {
    #[default]
    Function,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// Arguments en JSON texte, tels que produits par le modèle.
    pub arguments: String,
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
    #[error("budget du jour atteint ({0} requêtes)")]
    QuotaExceeded(u32),
}

/// Élément d'une réponse streamée.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    /// Fragment de texte, dans l'ordre.
    Text(String),
    /// Appels d'outils demandés, émis une seule fois, en fin de réponse.
    ToolCalls(Vec<ToolCall>),
}

pub type ChatStream = BoxStream<'static, Result<ChatEvent, LlmError>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Réponse streamée ; `tools` vide = pas d'appel d'outil possible.
    async fn chat_stream(
        &self,
        messages: Vec<Message>,
        tools: &[ToolSpec],
    ) -> Result<ChatStream, LlmError>;

    async fn complete(
        &self,
        messages: Vec<Message>,
        options: CompletionOptions,
    ) -> Result<String, LlmError>;
}
