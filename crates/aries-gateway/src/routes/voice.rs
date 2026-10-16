use std::sync::Arc;

use aries_voice::Voice;
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, header},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use crate::error::ApiError;

/// Environ 2 min de parole en Opus, ou 15 s en WAV non compressé.
pub const MAX_AUDIO_BYTES: usize = 4 * 1024 * 1024;

fn voice(state: &AppState) -> Result<&Arc<Voice>, ApiError> {
    state.voice.as_ref().ok_or_else(ApiError::voice_disabled)
}

/// Corps : l'enregistrement brut du micro, avec son `Content-Type` (ex. `audio/webm`).
pub async fn transcribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    audio: Bytes,
) -> Result<Json<Value>, ApiError> {
    let voice = voice(&state)?;
    if audio.is_empty() {
        return Err(ApiError::bad_request("enregistrement vide"));
    }
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");
    let text = voice.transcribe(audio, mime).await?;
    Ok(Json(json!({ "text": text })))
}

#[derive(Deserialize)]
pub struct SynthesisRequest {
    text: String,
}

/// Lit une phrase de la réponse : renvoie un WAV.
pub async fn synthesize(
    State(state): State<AppState>,
    Json(request): Json<SynthesisRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let voice = voice(&state)?;
    if request.text.trim().is_empty() {
        return Err(ApiError::bad_request("texte vide"));
    }
    let wav = voice.synthesize(&request.text).await?;
    Ok(([(header::CONTENT_TYPE, "audio/wav")], wav))
}
