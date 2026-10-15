//! Fournisseur principal (cloud) avec bascule automatique sur un secours (local).

use std::sync::Arc;

use async_trait::async_trait;

use crate::{ChatStream, CompletionOptions, LlmError, LlmProvider, Message, ToolSpec};

pub struct FallbackProvider {
    primary: Arc<dyn LlmProvider>,
    fallback: Arc<dyn LlmProvider>,
}

impl FallbackProvider {
    pub fn new(primary: Arc<dyn LlmProvider>, fallback: Arc<dyn LlmProvider>) -> Self {
        Self { primary, fallback }
    }
}

#[async_trait]
impl LlmProvider for FallbackProvider {
    /// Bascule seulement si le principal échoue avant de commencer à répondre
    /// (réseau, quota, panne) : une réponse déjà entamée n'est jamais mélangée.
    async fn chat_stream(
        &self,
        messages: Vec<Message>,
        tools: &[ToolSpec],
    ) -> Result<ChatStream, LlmError> {
        match self.primary.chat_stream(messages.clone(), tools).await {
            Ok(events) => Ok(events),
            Err(err) => {
                tracing::warn!(%err, "LLM principal indisponible, bascule sur le modèle local");
                self.fallback.chat_stream(messages, tools).await
            }
        }
    }

    async fn complete(
        &self,
        messages: Vec<Message>,
        options: CompletionOptions,
    ) -> Result<String, LlmError> {
        match self
            .primary
            .complete(messages.clone(), options.clone())
            .await
        {
            Ok(text) => Ok(text),
            Err(err) => {
                tracing::warn!(%err, "LLM principal indisponible, bascule sur le modèle local");
                self.fallback.complete(messages, options).await
            }
        }
    }
}
