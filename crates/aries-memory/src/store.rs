//! Accès SQLite : source de vérité de toute la mémoire.

use std::path::Path;

use aries_llm::Role;
use serde::Serialize;
use sqlx::{
    QueryBuilder, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::MemoryError;

const TITLE_MAX_CHARS: usize = 60;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Conversation {
    pub id: i64,
    pub title: Option<String>,
    /// Contient des données privées : traitée uniquement par le modèle local.
    pub private: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StoredMessage {
    pub id: i64,
    pub conversation_id: i64,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

impl StoredMessage {
    pub fn to_llm(&self) -> Option<aries_llm::Message> {
        Role::parse(&self.role).map(|role| aries_llm::Message::new(role, self.content.as_str()))
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Fact {
    pub id: i64,
    pub content: String,
    pub private: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Episode {
    pub id: i64,
    pub conversation_id: i64,
    pub summary: String,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub async fn open(path: &Path) -> Result<Self, MemoryError> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    // --- Conversations et messages (mémoire de travail) ---

    pub async fn create_conversation(&self) -> Result<Conversation, MemoryError> {
        Ok(sqlx::query_as(
            "INSERT INTO conversations DEFAULT VALUES RETURNING id, title, private, created_at, updated_at",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn conversation(&self, id: i64) -> Result<Option<Conversation>, MemoryError> {
        Ok(sqlx::query_as(
            "SELECT id, title, private, created_at, updated_at FROM conversations WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn list_conversations(&self, limit: i64) -> Result<Vec<Conversation>, MemoryError> {
        Ok(sqlx::query_as(
            "SELECT id, title, private, created_at, updated_at FROM conversations
             ORDER BY updated_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn mark_private(&self, conversation_id: i64) -> Result<(), MemoryError> {
        sqlx::query("UPDATE conversations SET private = 1 WHERE id = ?")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Ajoute un message ; le premier message utilisateur donne son titre à la conversation.
    pub async fn append_message(
        &self,
        conversation_id: i64,
        role: Role,
        content: &str,
    ) -> Result<i64, MemoryError> {
        let mut tx = self.pool.begin().await?;
        let id = sqlx::query_scalar(
            "INSERT INTO messages (conversation_id, role, content) VALUES (?, ?, ?) RETURNING id",
        )
        .bind(conversation_id)
        .bind(role.as_str())
        .bind(content)
        .fetch_one(&mut *tx)
        .await?;

        let title =
            (role == Role::User).then(|| content.chars().take(TITLE_MAX_CHARS).collect::<String>());
        sqlx::query(
            "UPDATE conversations SET title = COALESCE(title, ?),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(title)
        .bind(conversation_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn messages(&self, conversation_id: i64) -> Result<Vec<StoredMessage>, MemoryError> {
        Ok(sqlx::query_as(
            "SELECT id, conversation_id, role, content, created_at FROM messages
             WHERE conversation_id = ? ORDER BY id",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Les `limit` derniers messages, du plus récent au plus ancien.
    pub async fn latest_messages(
        &self,
        conversation_id: i64,
        limit: i64,
    ) -> Result<Vec<StoredMessage>, MemoryError> {
        Ok(sqlx::query_as(
            "SELECT id, conversation_id, role, content, created_at FROM messages
             WHERE conversation_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(conversation_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    // --- Consolidation ---

    /// Conversations à consolider : inactives depuis `idle_secs` ou avec `batch` messages en attente.
    pub async fn conversations_to_consolidate(
        &self,
        idle_secs: u64,
        batch: i64,
    ) -> Result<Vec<i64>, MemoryError> {
        Ok(sqlx::query_scalar(
            "SELECT conversation_id FROM messages WHERE consolidated = 0
             GROUP BY conversation_id
             HAVING MAX(created_at) <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?) OR COUNT(*) >= ?",
        )
        .bind(format!("-{idle_secs} seconds"))
        .bind(batch)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn pending_messages(
        &self,
        conversation_id: i64,
        limit: i64,
    ) -> Result<Vec<StoredMessage>, MemoryError> {
        Ok(sqlx::query_as(
            "SELECT id, conversation_id, role, content, created_at FROM messages
             WHERE conversation_id = ? AND consolidated = 0 ORDER BY id LIMIT ?",
        )
        .bind(conversation_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn mark_consolidated(
        &self,
        conversation_id: i64,
        up_to_id: i64,
    ) -> Result<(), MemoryError> {
        sqlx::query("UPDATE messages SET consolidated = 1 WHERE conversation_id = ? AND id <= ?")
            .bind(conversation_id)
            .bind(up_to_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // --- Faits (mémoire sémantique) ---

    pub async fn list_facts(&self) -> Result<Vec<Fact>, MemoryError> {
        Ok(sqlx::query_as(
            "SELECT id, content, private, created_at, updated_at FROM facts ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// `include_private = false` écarte les faits privés (réponse par un LLM cloud).
    pub async fn facts_by_ids(
        &self,
        ids: &[i64],
        include_private: bool,
    ) -> Result<Vec<Fact>, MemoryError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, content, private, created_at, updated_at FROM facts WHERE ",
        );
        if !include_private {
            query.push("private = 0 AND ");
        }
        query.push("id IN (");
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(*id);
        }
        separated.push_unseparated(")");
        Ok(query.build_query_as().fetch_all(&self.pool).await?)
    }

    pub async fn insert_fact(
        &self,
        content: &str,
        source_conversation_id: i64,
    ) -> Result<i64, MemoryError> {
        Ok(sqlx::query_scalar(
            "INSERT INTO facts (content, source_conversation_id, private)
             VALUES (?1, ?2, (SELECT private FROM conversations WHERE id = ?2)) RETURNING id",
        )
        .bind(content)
        .bind(source_conversation_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Un fait modifié depuis une conversation privée devient privé.
    pub async fn update_fact(
        &self,
        id: i64,
        content: &str,
        conversation_id: i64,
    ) -> Result<bool, MemoryError> {
        let result = sqlx::query(
            "UPDATE facts SET content = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                private = MAX(private, (SELECT private FROM conversations WHERE id = ?3))
             WHERE id = ?2",
        )
        .bind(content)
        .bind(id)
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_fact(&self, id: i64) -> Result<bool, MemoryError> {
        let result = sqlx::query("DELETE FROM facts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // --- Souvenirs (mémoire épisodique) ---

    pub async fn insert_episode(
        &self,
        conversation_id: i64,
        summary: &str,
        started_at: &str,
        ended_at: &str,
    ) -> Result<i64, MemoryError> {
        Ok(sqlx::query_scalar(
            "INSERT INTO episodes (conversation_id, summary, started_at, ended_at)
             VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(conversation_id)
        .bind(summary)
        .bind(started_at)
        .bind(ended_at)
        .fetch_one(&self.pool)
        .await?)
    }

    /// `include_private = false` écarte les souvenirs des conversations privées.
    pub async fn episodes_by_ids(
        &self,
        ids: &[i64],
        include_private: bool,
    ) -> Result<Vec<Episode>, MemoryError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, conversation_id, summary, started_at, ended_at FROM episodes WHERE ",
        );
        if !include_private {
            query.push(
                "conversation_id NOT IN (SELECT id FROM conversations WHERE private = 1) AND ",
            );
        }
        query.push("id IN (");
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(*id);
        }
        separated.push_unseparated(") ORDER BY ended_at");
        Ok(query.build_query_as().fetch_all(&self.pool).await?)
    }

    /// Souvenirs d'une conversation, dans l'ordre : sert de résumé de son début.
    pub async fn conversation_episodes(
        &self,
        conversation_id: i64,
    ) -> Result<Vec<Episode>, MemoryError> {
        Ok(sqlx::query_as(
            "SELECT id, conversation_id, summary, started_at, ended_at FROM episodes
             WHERE conversation_id = ? ORDER BY id",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn private_conversation_hides_its_facts_and_episodes() {
        let path = std::env::temp_dir().join(format!("aries-store-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Store::open(&path).await.unwrap();
        let public = store.create_conversation().await.unwrap().id;
        let secret = store.create_conversation().await.unwrap().id;
        store.mark_private(secret).await.unwrap();
        assert!(store.conversation(secret).await.unwrap().unwrap().private);

        let shared = store.insert_fact("aime le café", public).await.unwrap();
        let hidden = store
            .insert_fact("rendez-vous médical", secret)
            .await
            .unwrap();
        let public_episode = store.insert_episode(public, "a", "t0", "t1").await.unwrap();
        let secret_episode = store.insert_episode(secret, "b", "t0", "t1").await.unwrap();

        let ids = |facts: Vec<Fact>| facts.into_iter().map(|f| f.id).collect::<Vec<_>>();
        let all = [shared, hidden];
        assert_eq!(
            ids(store.facts_by_ids(&all, false).await.unwrap()),
            [shared]
        );
        assert_eq!(store.facts_by_ids(&all, true).await.unwrap().len(), 2);

        // Un fait public modifié depuis une conversation privée devient privé.
        assert!(
            store
                .update_fact(shared, "aime le thé", secret)
                .await
                .unwrap()
        );
        assert!(store.facts_by_ids(&all, false).await.unwrap().is_empty());

        let episodes = [public_episode, secret_episode];
        let visible = store.episodes_by_ids(&episodes, false).await.unwrap();
        assert_eq!(
            visible.iter().map(|e| e.id).collect::<Vec<_>>(),
            [public_episode]
        );
        assert_eq!(
            store.episodes_by_ids(&episodes, true).await.unwrap().len(),
            2
        );
        let _ = std::fs::remove_file(&path);
    }
}
