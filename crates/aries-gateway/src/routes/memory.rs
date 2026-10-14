use aries_memory::Fact;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use super::AppState;
use crate::error::ApiError;

pub async fn list_facts(State(state): State<AppState>) -> Result<Json<Vec<Fact>>, ApiError> {
    Ok(Json(state.memory.list_facts().await?))
}

pub async fn delete_fact(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if state.memory.delete_fact(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("fait inconnu"))
    }
}
