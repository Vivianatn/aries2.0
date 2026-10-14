//! Mémoire d'ArIès, sur trois niveaux :
//! - travail : la conversation en cours (fenêtre glissante + résumé du début) ;
//! - sémantique : faits durables sur l'utilisateur ;
//! - épisodique : résumés horodatés des conversations passées.
//!
//! SQLite est la source de vérité ; Qdrant indexe les vecteurs (ids identiques).

mod activity;
mod config;
mod consolidation;
mod recall;
mod store;
mod vectors;

pub use activity::{ActivityGauge, ActivityGuard};
pub use config::MemoryConfig;
pub use store::{Conversation, Episode, Fact, StoredMessage};

use std::time::Duration;

use aries_llm::{LlmError, OpenAiCompatEmbedder, Role};

use crate::{
    store::Store,
    vectors::{EPISODES, FACTS, VectorIndex},
};

const CONVERSATIONS_LIST_LIMIT: i64 = 50;
const QDRANT_CONNECT_ATTEMPTS: u32 = 10;

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("base SQLite : {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration SQLite : {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("Qdrant : {0}")]
    Vectors(#[from] reqwest::Error),
    #[error("LLM ou embeddings : {0}")]
    Llm(#[from] LlmError),
    #[error("fichier : {0}")]
    Io(#[from] std::io::Error),
}

pub struct Memory {
    store: Store,
    vectors: VectorIndex,
    embedder: OpenAiCompatEmbedder,
    config: MemoryConfig,
}

impl Memory {
    pub async fn connect(config: MemoryConfig) -> Result<Self, MemoryError> {
        let store = Store::open(&config.db_path).await?;
        let vectors = VectorIndex::new(&config.qdrant_url)?;
        let embedder =
            OpenAiCompatEmbedder::new(&config.embedding_base_url, &config.embedding_model)?;

        // Qdrant peut démarrer après ArIès : on lui laisse un peu de temps.
        let mut attempt = 1;
        while let Err(err) = ensure_collections(&vectors, config.embedding_dim).await {
            if attempt >= QDRANT_CONNECT_ATTEMPTS {
                return Err(err);
            }
            tracing::warn!(%err, attempt, "Qdrant pas encore prêt, nouvel essai");
            attempt += 1;
            tokio::time::sleep(Duration::from_secs(3)).await;
        }

        Ok(Self {
            store,
            vectors,
            embedder,
            config,
        })
    }

    pub async fn create_conversation(&self) -> Result<Conversation, MemoryError> {
        self.store.create_conversation().await
    }

    pub async fn conversation(&self, id: i64) -> Result<Option<Conversation>, MemoryError> {
        self.store.conversation(id).await
    }

    pub async fn list_conversations(&self) -> Result<Vec<Conversation>, MemoryError> {
        self.store
            .list_conversations(CONVERSATIONS_LIST_LIMIT)
            .await
    }

    pub async fn messages(&self, conversation_id: i64) -> Result<Vec<StoredMessage>, MemoryError> {
        self.store.messages(conversation_id).await
    }

    pub async fn append_message(
        &self,
        conversation_id: i64,
        role: Role,
        content: &str,
    ) -> Result<i64, MemoryError> {
        self.store
            .append_message(conversation_id, role, content)
            .await
    }

    pub async fn list_facts(&self) -> Result<Vec<Fact>, MemoryError> {
        self.store.list_facts().await
    }

    /// Oublie un fait (SQLite + index vectoriel). `false` s'il n'existait pas.
    pub async fn delete_fact(&self, id: i64) -> Result<bool, MemoryError> {
        let deleted = self.store.delete_fact(id).await?;
        if deleted {
            self.vectors.delete(FACTS, &[id as u64]).await?;
        }
        Ok(deleted)
    }
}

async fn ensure_collections(vectors: &VectorIndex, dim: usize) -> Result<(), MemoryError> {
    vectors.ensure_collection(FACTS, dim).await?;
    vectors.ensure_collection(EPISODES, dim).await
}

/// Coupe `text` à `max` caractères (sans casser un caractère UTF-8).
pub(crate) fn truncate_chars(text: &str, max: usize) -> &str {
    text.char_indices()
        .nth(max)
        .map_or(text, |(index, _)| &text[..index])
}

pub(crate) fn ids_u64(ids: &[i64]) -> Vec<u64> {
    ids.iter().map(|&id| id as u64).collect()
}
