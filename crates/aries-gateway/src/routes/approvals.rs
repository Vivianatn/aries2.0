use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use super::AppState;
use crate::error::ApiError;

#[derive(Deserialize)]
pub struct ApprovalDecision {
    approved: bool,
}

/// Réponse de l'utilisateur à une action `danger` (événement SSE `approval_required`).
pub async fn decide(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(decision): Json<ApprovalDecision>,
) -> Result<StatusCode, ApiError> {
    if state.agent.approvals().resolve(id, decision.approved) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("validation inconnue ou expirée"))
    }
}
