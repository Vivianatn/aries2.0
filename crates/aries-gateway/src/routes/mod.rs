mod approvals;
mod conversations;
mod memory;

use std::{path::Path, sync::Arc};

use aries_core::Agent;
use aries_memory::{ActivityGauge, Memory};
use axum::{
    Json, Router,
    routing::{delete, get, put},
};
use serde_json::{Value, json};
use tower_http::{services::ServeDir, trace::TraceLayer};

#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<Agent>,
    pub memory: Arc<Memory>,
    pub activity: ActivityGauge,
    pub persona: Arc<str>,
}

pub fn router(state: AppState, web_dir: &Path) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route(
            "/api/v1/conversations",
            get(conversations::list).post(conversations::create),
        )
        .route(
            "/api/v1/conversations/{id}/messages",
            get(conversations::messages).post(conversations::post_message),
        )
        .route("/api/v1/memory-facts", get(memory::list_facts))
        .route("/api/v1/memory-facts/{id}", delete(memory::delete_fact))
        .route("/api/v1/tool-approvals/{id}", put(approvals::decide))
        .fallback_service(ServeDir::new(web_dir))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}
