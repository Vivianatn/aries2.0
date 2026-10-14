//! Fournisseur pour toute API compatible OpenAI : llama.cpp, mistral.rs, Gemini, OpenAI…

use std::time::Duration;

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::{StreamExt, TryStreamExt, future};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{CompletionOptions, LlmError, LlmProvider, Message, TokenStream};

const STREAM_END_MARKER: &str = "[DONE]";

fn http_client() -> Result<reqwest::Client, LlmError> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()?)
}

fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}/{path}", base_url.trim_end_matches('/'))
}

/// Renvoie la réponse si le statut est un succès, sinon une erreur avec le corps.
async fn check_status(response: reqwest::Response) -> Result<reqwest::Response, LlmError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(LlmError::Status { status, body })
}

pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    chat_url: String,
    model: String,
    api_key: Option<String>,
    reasoning_effort: Option<String>,
}

impl OpenAiCompatProvider {
    pub fn new(
        base_url: &str,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Result<Self, LlmError> {
        Ok(Self {
            client: http_client()?,
            chat_url: endpoint(base_url, "chat/completions"),
            model: model.into(),
            api_key,
            reasoning_effort: None,
        })
    }

    /// Effort de « réflexion » des modèles qui raisonnent (ex. `none` pour Gemini Flash).
    pub fn with_reasoning_effort(mut self, effort: Option<String>) -> Self {
        self.reasoning_effort = effort;
        self
    }

    async fn send(&self, body: &ChatRequest<'_>) -> Result<reqwest::Response, LlmError> {
        let mut request = self.client.post(&self.chat_url).json(body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        check_status(request.send().await?).await
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
}

#[derive(Deserialize)]
struct ChatChunk {
    choices: Vec<ChunkChoice>,
}

#[derive(Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: ChunkDelta,
}

#[derive(Default, Deserialize)]
struct ChunkDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatCompletion {
    choices: Vec<CompletionChoice>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    message: CompletionMessage,
}

#[derive(Deserialize)]
struct CompletionMessage {
    content: Option<String>,
}

#[async_trait]
impl LlmProvider for OpenAiCompatProvider {
    async fn chat_stream(&self, messages: Vec<Message>) -> Result<TokenStream, LlmError> {
        let body = ChatRequest {
            model: &self.model,
            messages: &messages,
            stream: true,
            max_tokens: None,
            temperature: None,
            response_format: None,
            reasoning_effort: self.reasoning_effort.as_deref(),
        };
        let response = self.send(&body).await?;

        let tokens = response
            .bytes_stream()
            .eventsource()
            .map_err(|err| LlmError::Stream(err.to_string()))
            .try_take_while(|event| future::ready(Ok(event.data != STREAM_END_MARKER)))
            .try_filter_map(|event| async move {
                let chunk: ChatChunk = serde_json::from_str(&event.data)?;
                let text = chunk
                    .choices
                    .into_iter()
                    .next()
                    .and_then(|choice| choice.delta.content);
                Ok::<_, LlmError>(text.filter(|text| !text.is_empty()))
            });

        Ok(tokens.boxed())
    }

    async fn complete(
        &self,
        messages: Vec<Message>,
        options: CompletionOptions,
    ) -> Result<String, LlmError> {
        let body = ChatRequest {
            model: &self.model,
            messages: &messages,
            stream: false,
            max_tokens: options.max_tokens,
            temperature: options.temperature,
            response_format: options.json.then(|| json!({ "type": "json_object" })),
            reasoning_effort: self.reasoning_effort.as_deref(),
        };
        let completion: ChatCompletion = self.send(&body).await?.json().await?;
        Ok(completion
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .unwrap_or_default())
    }
}

/// Embeddings via `/embeddings` (llama.cpp lancé avec `--embeddings`, OpenAI…).
pub struct OpenAiCompatEmbedder {
    client: reqwest::Client,
    url: String,
    model: String,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    index: usize,
    embedding: Vec<f32>,
}

impl OpenAiCompatEmbedder {
    pub fn new(base_url: &str, model: impl Into<String>) -> Result<Self, LlmError> {
        Ok(Self {
            client: http_client()?,
            url: endpoint(base_url, "embeddings"),
            model: model.into(),
        })
    }

    /// Un vecteur par texte, dans le même ordre que `inputs`.
    pub async fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let request = self
            .client
            .post(&self.url)
            .json(&json!({ "model": self.model, "input": inputs }));
        let mut response: EmbeddingResponse =
            check_status(request.send().await?).await?.json().await?;
        response.data.sort_unstable_by_key(|item| item.index);
        Ok(response
            .data
            .into_iter()
            .map(|item| item.embedding)
            .collect())
    }

    pub async fn embed_one(&self, input: &str) -> Result<Vec<f32>, LlmError> {
        self.embed(&[input])
            .await?
            .pop()
            .ok_or_else(|| LlmError::Stream("réponse d'embedding vide".into()))
    }
}
