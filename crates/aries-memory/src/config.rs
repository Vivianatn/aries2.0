use std::path::PathBuf;

use serde::Deserialize;

/// Section `[memory]` de `config/aries.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub db_path: PathBuf,
    pub qdrant_url: String,
    pub embedding_base_url: String,
    pub embedding_model: String,
    pub embedding_dim: usize,
    /// Nombre max de faits et score de similarité minimal (cosinus) pour les rappeler.
    pub facts_limit: usize,
    pub facts_min_score: f32,
    pub episodes_limit: usize,
    pub episodes_min_score: f32,
    /// Taille max (en caractères) de l'historique envoyé au LLM.
    pub history_char_budget: usize,
    /// Une conversation est consolidée après ce délai d'inactivité…
    pub consolidation_idle_secs: u64,
    /// … ou dès qu'elle accumule ce nombre de messages non traités.
    pub consolidation_batch: i64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            db_path: "data/aries.db".into(),
            qdrant_url: "http://qdrant:6333".into(),
            embedding_base_url: "http://embeddings:8080/v1".into(),
            embedding_model: "bge-m3".into(),
            embedding_dim: 1024,
            facts_limit: 8,
            facts_min_score: 0.45,
            episodes_limit: 3,
            episodes_min_score: 0.5,
            history_char_budget: 12_000,
            consolidation_idle_secs: 120,
            consolidation_batch: 20,
        }
    }
}
