//! Client MCP : chaque outil d'un serveur MCP (HTTP « streamable ») devient un `Tool`.
//! Les serveurs tournent dans des conteneurs séparés ; aucun n'est lancé par ArIès.

use std::{collections::HashMap, env, sync::Arc, time::Duration};

use aries_governance::RiskLevel;
use aries_llm::ToolSpec;
use async_trait::async_trait;
use rmcp::{
    ServiceError, ServiceExt,
    model::{CallToolRequestParams, ContentBlock, ResourceContents, Tool as McpToolInfo},
    service::{RoleClient, RunningService},
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::{Tool, ToolError, truncate_chars};

const CONNECT_ATTEMPTS: u32 = 15;
const CONNECT_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Un serveur MCP dans `[[tools.mcp]]`.
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    /// Point d'accès HTTP, ex. `http://playwright:8931/mcp`.
    pub url: String,
    /// Variable d'environnement (`.env`) qui contient un jeton `Authorization: Bearer`.
    pub token_env: Option<String>,
    /// Outils exposés au LLM (vide = tous) : chaque outil coûte du contexte.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Niveau de risque imposé par outil ; sinon déduit des annotations MCP.
    #[serde(default)]
    pub risks: HashMap<String, RiskLevel>,
    /// Outils dont le résultat est privé : jamais envoyé à un LLM cloud.
    #[serde(default)]
    pub private: Vec<String>,
    /// Outils proposés aussi au modèle local dans une conversation privée.
    #[serde(default)]
    pub local: Vec<String>,
    /// Descriptions remplacées par outil, pour mieux guider le LLM.
    #[serde(default)]
    pub descriptions: HashMap<String, String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Texte maximal renvoyé au LLM par appel.
    #[serde(default = "default_output_max_chars")]
    pub output_max_chars: usize,
}

fn default_timeout_secs() -> u64 {
    60
}

fn default_output_max_chars() -> usize {
    8_000
}

type Client = RunningService<RoleClient, ()>;

/// Connexion partagée par les outils d'un serveur, rétablie si elle tombe.
pub(crate) struct McpConnection {
    config: McpServerConfig,
    client: RwLock<Client>,
}

impl McpConnection {
    /// Réessaie pendant ~30 s : le conteneur du serveur peut démarrer après ArIès.
    pub(crate) async fn connect(config: McpServerConfig) -> Result<Arc<Self>, ToolError> {
        let mut attempt = 1;
        let client = loop {
            match open(&config).await {
                Ok(client) => break client,
                Err(err) if attempt < CONNECT_ATTEMPTS => {
                    tracing::debug!(server = %config.name, %err, attempt, "serveur MCP pas prêt");
                    attempt += 1;
                    tokio::time::sleep(CONNECT_RETRY_DELAY).await;
                }
                Err(err) => return Err(err),
            }
        };
        Ok(Arc::new(Self {
            config,
            client: RwLock::new(client),
        }))
    }

    /// Outils du serveur retenus par la configuration, prêts à enregistrer.
    pub(crate) async fn tools(self: &Arc<Self>) -> Result<Vec<McpTool>, ToolError> {
        let listed = self
            .client
            .read()
            .await
            .list_all_tools()
            .await
            .map_err(service_error)?;
        Ok(listed
            .into_iter()
            .filter(|info| {
                self.config.tools.is_empty() || self.config.tools.iter().any(|t| t == &info.name)
            })
            .map(|info| McpTool {
                private: self.config.private.iter().any(|t| t == &info.name),
                local: self.config.local.iter().any(|t| t == &info.name),
                risk: self
                    .config
                    .risks
                    .get(info.name.as_ref())
                    .copied()
                    .unwrap_or_else(|| risk_from_annotations(&info)),
                spec: spec_of(&info, self.config.descriptions.get(info.name.as_ref())),
                connection: self.clone(),
            })
            .collect())
    }

    async fn call(&self, name: &str, arguments: Value) -> Result<String, ToolError> {
        let Value::Object(arguments) = arguments else {
            return Err(ToolError::InvalidArguments("objet JSON attendu".into()));
        };
        let params = CallToolRequestParams::new(name.to_owned()).with_arguments(arguments);
        let timeout = Duration::from_secs(self.config.timeout_secs);

        let first = self.call_once(params.clone(), timeout).await;
        let result = match first {
            Err(ServiceError::TransportClosed | ServiceError::TransportSend(_)) => {
                tracing::warn!(server = %self.config.name, "connexion MCP perdue, reconnexion");
                *self.client.write().await = open(&self.config).await?;
                self.call_once(params, timeout).await
            }
            other => other,
        }
        .map_err(service_error)?;

        let text = result
            .content
            .iter()
            .filter_map(content_text)
            .collect::<Vec<_>>()
            .join("\n");
        let text = truncate_chars(&text, self.config.output_max_chars).to_owned();
        if result.is_error == Some(true) {
            Err(ToolError::Failed(text))
        } else {
            Ok(text)
        }
    }

    async fn call_once(
        &self,
        params: CallToolRequestParams,
        timeout: Duration,
    ) -> Result<rmcp::model::CallToolResult, ServiceError> {
        let client = self.client.read().await;
        tokio::time::timeout(timeout, client.call_tool(params))
            .await
            .unwrap_or(Err(ServiceError::Timeout { timeout }))
    }
}

async fn open(config: &McpServerConfig) -> Result<Client, ToolError> {
    let mut transport = StreamableHttpClientTransportConfig::with_uri(config.url.as_str());
    if let Some(token) = config
        .token_env
        .as_deref()
        .and_then(|var| env::var(var).ok())
        .filter(|token| !token.is_empty())
    {
        transport = transport.auth_header(token);
    }
    ().serve(StreamableHttpClientTransport::from_config(transport))
        .await
        .map_err(|err| ToolError::Failed(format!("serveur MCP {} : {err}", config.name)))
}

pub struct McpTool {
    spec: ToolSpec,
    risk: RiskLevel,
    private: bool,
    local: bool,
    connection: Arc<McpConnection>,
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn risk(&self) -> RiskLevel {
        self.risk
    }

    fn private(&self) -> bool {
        self.private
    }

    fn local(&self) -> bool {
        self.local
    }

    async fn call(&self, arguments: Value) -> Result<String, ToolError> {
        self.connection.call(&self.spec.name, arguments).await
    }
}

/// Spécification MCP : `readOnlyHint` → read ; `destructiveHint: false` → action ;
/// sinon (destructif par défaut) → danger.
fn risk_from_annotations(info: &McpToolInfo) -> RiskLevel {
    let annotations = info.annotations.as_ref();
    if annotations.and_then(|a| a.read_only_hint) == Some(true) {
        RiskLevel::Read
    } else if annotations.and_then(|a| a.destructive_hint) == Some(false) {
        RiskLevel::Action
    } else {
        RiskLevel::Danger
    }
}

fn spec_of(info: &McpToolInfo, description: Option<&String>) -> ToolSpec {
    let mut parameters = (*info.input_schema).clone();
    // Mot-clé refusé par certains fournisseurs (Gemini).
    parameters.remove("$schema");
    ToolSpec {
        name: info.name.to_string(),
        description: description
            .map(String::as_str)
            .or(info.description.as_deref())
            .or(info.title.as_deref())
            .unwrap_or_default()
            .to_owned(),
        parameters: Value::Object(parameters),
    }
}

fn content_text(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(text) => Some(text.text.clone()),
        ContentBlock::Resource(resource) => match &resource.resource {
            ResourceContents::TextResourceContents { text, .. } => Some(text.clone()),
            _ => None,
        },
        ContentBlock::Image(_) => Some("[image non transmise]".into()),
        _ => None,
    }
}

fn service_error(err: ServiceError) -> ToolError {
    ToolError::Failed(format!("MCP : {err}"))
}

#[cfg(test)]
mod tests {
    use rmcp::model::ToolAnnotations;
    use serde_json::json;

    use super::*;

    fn info(annotations: Option<ToolAnnotations>) -> McpToolInfo {
        let mut info: McpToolInfo = serde_json::from_value(json!({
            "name": "browser_navigate",
            "description": "Ouvre une URL",
            "inputSchema": {
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "properties": { "url": { "type": "string" } }
            }
        }))
        .unwrap();
        info.annotations = annotations;
        info
    }

    fn annotations(read_only: Option<bool>, destructive: Option<bool>) -> ToolAnnotations {
        serde_json::from_value(json!({
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
        }))
        .unwrap()
    }

    #[test]
    fn risk_follows_mcp_annotations() {
        assert_eq!(risk_from_annotations(&info(None)), RiskLevel::Danger);
        let read = info(Some(annotations(Some(true), None)));
        assert_eq!(risk_from_annotations(&read), RiskLevel::Read);
        let action = info(Some(annotations(Some(false), Some(false))));
        assert_eq!(risk_from_annotations(&action), RiskLevel::Action);
        let danger = info(Some(annotations(Some(false), Some(true))));
        assert_eq!(risk_from_annotations(&danger), RiskLevel::Danger);
    }

    #[test]
    fn spec_drops_schema_keyword() {
        let spec = spec_of(&info(None), None);
        assert_eq!(spec.name, "browser_navigate");
        assert_eq!(spec.description, "Ouvre une URL");
        let replaced = "Ouvre une page dynamique".to_owned();
        assert_eq!(spec_of(&info(None), Some(&replaced)).description, replaced);
        assert!(spec.parameters.get("$schema").is_none());
        assert_eq!(spec.parameters["type"], "object");
    }
}
