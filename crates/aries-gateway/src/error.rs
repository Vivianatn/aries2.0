use aries_llm::LlmError;
use aries_memory::MemoryError;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// Erreur d'API au format `{ "error": { "code", "message" } }`.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }
}

impl From<LlmError> for ApiError {
    fn from(err: LlmError) -> Self {
        tracing::error!(%err, "LLM indisponible");
        Self::new(StatusCode::BAD_GATEWAY, "llm_unavailable", err.to_string())
    }
}

impl From<MemoryError> for ApiError {
    fn from(err: MemoryError) -> Self {
        tracing::error!(%err, "erreur de mémoire");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "memory_error",
            err.to_string(),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": { "code": self.code, "message": self.message } });
        (self.status, Json(body)).into_response()
    }
}
