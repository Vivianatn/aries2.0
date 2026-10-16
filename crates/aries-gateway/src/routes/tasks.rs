use aries_proactive::{Notification, Task};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use super::AppState;
use crate::error::ApiError;

const NOTIFICATIONS_LIMIT: i64 = 30;

/// Tâches et rappels encore planifiés, par prochaine échéance.
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<Task>>, ApiError> {
    Ok(Json(state.tasks.active().await?))
}

pub async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if state.tasks.cancel(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("tâche inconnue ou déjà terminée"))
    }
}

/// Messages envoyés par les tâches de fond, du plus récent au plus ancien.
pub async fn notifications(
    State(state): State<AppState>,
) -> Result<Json<Vec<Notification>>, ApiError> {
    Ok(Json(state.tasks.notifications(NOTIFICATIONS_LIMIT).await?))
}
