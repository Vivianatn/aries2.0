use std::convert::Infallible;

use aries_llm::Role;
use aries_memory::{Conversation, StoredMessage};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::{Stream, StreamExt, stream};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use super::AppState;
use crate::error::ApiError;

const EVENT_BUFFER: usize = 64;

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<Conversation>>, ApiError> {
    Ok(Json(state.memory.list_conversations().await?))
}

pub async fn create(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<Conversation>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(state.memory.create_conversation().await?),
    ))
}

pub async fn messages(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<StoredMessage>>, ApiError> {
    ensure_exists(&state, id).await?;
    Ok(Json(state.memory.messages(id).await?))
}

#[derive(Deserialize)]
pub struct PostMessage {
    content: String,
}

/// Enregistre le message, puis répond en SSE : `token` (`{"text"}`), éventuellement
/// `error`, puis `done` (`{"message_id"}`). La génération continue même si le client
/// se déconnecte : la réponse est toujours enregistrée.
pub async fn post_message(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(request): Json<PostMessage>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let content = request.content.trim();
    if content.is_empty() {
        return Err(ApiError::bad_request("le message est vide"));
    }
    ensure_exists(&state, id).await?;

    let guard = state.activity.begin();
    state.memory.append_message(id, Role::User, content).await?;
    let prompt = state.memory.build_prompt(id, &state.persona).await?;
    let mut tokens = state.llm.chat_stream(prompt).await?;

    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    let memory = state.memory.clone();
    tokio::spawn(async move {
        let _guard = guard;
        let mut answer = String::new();
        while let Some(item) = tokens.next().await {
            match item {
                Ok(text) => {
                    answer.push_str(&text);
                    let _ = tx.send(json_event("token", json!({ "text": text }))).await;
                }
                Err(err) => {
                    tracing::warn!(%err, "flux LLM interrompu");
                    let _ = tx
                        .send(json_event("error", json!({ "message": err.to_string() })))
                        .await;
                    break;
                }
            }
        }

        let message_id = if answer.trim().is_empty() {
            None
        } else {
            memory
                .append_message(id, Role::Assistant, &answer)
                .await
                .inspect_err(|err| tracing::error!(%err, "réponse non enregistrée"))
                .ok()
        };
        let _ = tx
            .send(json_event("done", json!({ "message_id": message_id })))
            .await;
    });

    let events = stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|event| (Ok::<_, Infallible>(event), rx))
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

async fn ensure_exists(state: &AppState, id: i64) -> Result<(), ApiError> {
    match state.memory.conversation(id).await? {
        Some(_) => Ok(()),
        None => Err(ApiError::not_found("conversation inconnue")),
    }
}

fn json_event(name: &str, data: serde_json::Value) -> Event {
    Event::default().event(name).data(data.to_string())
}
