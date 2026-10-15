use std::{env, fs, net::SocketAddr, path::PathBuf};

use anyhow::Context;
use aries_governance::GovernanceConfig;
use aries_memory::MemoryConfig;
use aries_proactive::ProactiveConfig;
use aries_tools::ToolsConfig;
use serde::Deserialize;

const DEFAULT_CONFIG_PATH: &str = "config/aries.toml";

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub assistant: AssistantConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub governance: GovernanceConfig,
    #[serde(default)]
    pub proactive: ProactiveConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub web_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct AssistantConfig {
    pub persona_path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct LlmConfig {
    /// Modèle local (llama.cpp) : secours du chat et tâches de fond (mémoire).
    pub local: LocalLlmConfig,
    /// Modèle cloud optionnel pour le chat, actif seulement si sa clé API est définie.
    pub cloud: Option<CloudLlmConfig>,
}

#[derive(Debug, Deserialize)]
pub struct LocalLlmConfig {
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct CloudLlmConfig {
    pub base_url: String,
    pub model: String,
    /// Nom de la variable d'environnement (dans `.env`) qui contient la clé API.
    pub api_key_env: String,
    pub reasoning_effort: Option<String>,
    /// Budget : requêtes cloud max par jour (chaque tour d'outil compte), puis modèle local.
    pub daily_request_limit: Option<u32>,
}

impl Config {
    /// Lit `ARIES_CONFIG` (ou `config/aries.toml`), puis applique la surcharge `ARIES_BIND`.
    pub fn load() -> anyhow::Result<Self> {
        let path = env::var("ARIES_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.into());
        let raw = fs::read_to_string(&path).with_context(|| format!("lecture de {path}"))?;
        let mut config: Self =
            toml::from_str(&raw).with_context(|| format!("format invalide dans {path}"))?;

        if let Ok(bind) = env::var("ARIES_BIND") {
            config.server.bind = bind.parse().context("ARIES_BIND invalide")?;
        }
        Ok(config)
    }
}
