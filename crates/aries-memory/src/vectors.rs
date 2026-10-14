//! Client minimal de l'API REST de Qdrant (pas besoin du client gRPC complet).

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::MemoryError;

pub const FACTS: &str = "facts";
pub const EPISODES: &str = "episodes";

pub struct VectorIndex {
    client: reqwest::Client,
    base_url: String,
}

#[derive(Deserialize)]
struct QdrantResponse<T> {
    result: T,
}

#[derive(Deserialize)]
struct Exists {
    exists: bool,
}

#[derive(Deserialize)]
struct QueryResult {
    points: Vec<ScoredPoint>,
}

#[derive(Deserialize)]
struct ScoredPoint {
    id: u64,
}

impl VectorIndex {
    pub fn new(base_url: &str) -> Result<Self, MemoryError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(30))
                .build()?,
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/collections/{path}", self.base_url)
    }

    /// Crée la collection (distance cosinus) si elle n'existe pas encore.
    pub async fn ensure_collection(&self, name: &str, dim: usize) -> Result<(), MemoryError> {
        let exists: QdrantResponse<Exists> = self
            .client
            .get(self.url(&format!("{name}/exists")))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if !exists.result.exists {
            self.client
                .put(self.url(name))
                .json(&json!({ "vectors": { "size": dim, "distance": "Cosine" } }))
                .send()
                .await?
                .error_for_status()?;
            tracing::info!(collection = name, dim, "collection Qdrant créée");
        }
        Ok(())
    }

    pub async fn upsert(
        &self,
        collection: &str,
        points: impl IntoIterator<Item = (u64, Vec<f32>, Value)>,
    ) -> Result<(), MemoryError> {
        let points: Vec<Value> = points
            .into_iter()
            .map(|(id, vector, payload)| json!({ "id": id, "vector": vector, "payload": payload }))
            .collect();
        if points.is_empty() {
            return Ok(());
        }
        self.client
            .put(self.url(&format!("{collection}/points?wait=true")))
            .json(&json!({ "points": points }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn delete(&self, collection: &str, ids: &[u64]) -> Result<(), MemoryError> {
        if ids.is_empty() {
            return Ok(());
        }
        self.client
            .post(self.url(&format!("{collection}/points/delete?wait=true")))
            .json(&json!({ "points": ids }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Ids des `limit` points les plus proches de `vector` au-dessus de `min_score`,
    /// du plus proche au plus lointain.
    pub async fn search(
        &self,
        collection: &str,
        vector: &[f32],
        limit: usize,
        min_score: f32,
        filter: Option<Value>,
    ) -> Result<Vec<i64>, MemoryError> {
        let mut body = json!({ "query": vector, "limit": limit, "score_threshold": min_score });
        if let Some(filter) = filter {
            body["filter"] = filter;
        }
        let response: QdrantResponse<QueryResult> = self
            .client
            .post(self.url(&format!("{collection}/points/query")))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response
            .result
            .points
            .into_iter()
            .map(|point| point.id as i64)
            .collect())
    }
}
