//! Outils qu'ArIès peut appeler. Chacun déclare son niveau de risque (gouvernance).

mod web_page;
mod web_search;

pub use web_page::ReadWebPage;
pub use web_search::WebSearch;

use std::sync::Arc;

use aries_governance::RiskLevel;
use aries_llm::ToolSpec;
use async_trait::async_trait;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("arguments invalides : {0}")]
    InvalidArguments(String),
    #[error("requête HTTP échouée : {0}")]
    Http(#[from] reqwest::Error),
    #[error("le site a répondu {0}")]
    Status(reqwest::StatusCode),
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Failed(String),
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn risk(&self) -> RiskLevel;
    /// Résultat en texte, destiné au LLM.
    async fn call(&self, arguments: Value) -> Result<String, ToolError>;
}

/// Section `[tools]` de `config/aries.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    /// Instance SearXNG (format JSON activé).
    pub searxng_url: String,
    pub search_results: usize,
    /// Texte maximal renvoyé au LLM pour une page (le contexte local est de 8k tokens).
    pub page_max_chars: usize,
    /// Allers-retours outils ↔ LLM par message, avant la réponse finale forcée.
    pub max_rounds: usize,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            searxng_url: "http://searxng:8080".into(),
            search_results: 5,
            page_max_chars: 6_000,
            max_rounds: 5,
        }
    }
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<(ToolSpec, Arc<dyn Tool>)>,
}

impl ToolRegistry {
    /// Outils disponibles sans compte ni clé : recherche web et lecture de pages.
    pub fn with_web_tools(config: &ToolsConfig) -> Result<Self, ToolError> {
        let mut registry = Self::default();
        registry.register(WebSearch::new(&config.searxng_url, config.search_results)?);
        registry.register(ReadWebPage::new(config.page_max_chars));
        Ok(registry)
    }

    pub fn register(&mut self, tool: impl Tool + 'static) {
        self.tools.push((tool.spec(), Arc::new(tool)));
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|(spec, _)| spec.clone()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools
            .iter()
            .find(|(spec, _)| spec.name == name)
            .map(|(_, tool)| tool)
    }
}

pub(crate) fn parse_arguments<T: DeserializeOwned>(arguments: Value) -> Result<T, ToolError> {
    serde_json::from_value(arguments).map_err(|err| ToolError::InvalidArguments(err.to_string()))
}

/// Coupe `text` à `max` caractères (sans casser un caractère UTF-8).
pub(crate) fn truncate_chars(text: &str, max: usize) -> &str {
    text.char_indices()
        .nth(max)
        .map_or(text, |(index, _)| &text[..index])
}
