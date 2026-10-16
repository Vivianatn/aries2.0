mod approvals;
mod conversations;
mod memory;
mod tasks;
mod voice;

use std::{path::Path, sync::Arc};

use aries_core::Agent;
use aries_memory::{ActivityGauge, Memory};
use aries_proactive::TaskStore;
use aries_voice::Voice;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderName, HeaderValue, header},
    routing::{delete, get, post, put},
};
use serde_json::{Value, json};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer, trace::TraceLayer};

/// L'interface ne charge rien d'extérieur (police auto-hébergée, aucun traceur) : le
/// navigateur bloque toute requête vers un tiers. Micro autorisé pour ce site seulement.
const SECURITY_HEADERS: [(HeaderName, &str); 5] = [
    (
        header::CONTENT_SECURITY_POLICY,
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; media-src 'self' blob:; connect-src 'self'; \
         font-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; \
         form-action 'self'",
    ),
    (header::REFERRER_POLICY, "no-referrer"),
    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    (header::X_FRAME_OPTIONS, "DENY"),
    (
        HeaderName::from_static("permissions-policy"),
        "microphone=(self), camera=(), geolocation=(), payment=(), usb=()",
    ),
];

#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<Agent>,
    pub memory: Arc<Memory>,
    pub activity: ActivityGauge,
    pub persona: Arc<str>,
    pub voice: Option<Arc<Voice>>,
    pub tasks: TaskStore,
    /// Modèle cloud actif (transparence : l'interface dit où partent les messages).
    pub cloud_model: Option<String>,
}

pub fn router(state: AppState, web_dir: &Path) -> Router {
    let router = Router::new()
        .route("/api/v1/health", get(health))
        .route(
            "/api/v1/conversations",
            get(conversations::list).post(conversations::create),
        )
        .route(
            "/api/v1/conversations/{id}",
            get(conversations::get).delete(conversations::delete),
        )
        .route(
            "/api/v1/conversations/{id}/messages",
            get(conversations::messages).post(conversations::post_message),
        )
        .route("/api/v1/memory-facts", get(memory::list_facts))
        .route("/api/v1/memory-facts/{id}", delete(memory::delete_fact))
        .route("/api/v1/tasks", get(tasks::list))
        .route("/api/v1/tasks/{id}", delete(tasks::cancel))
        .route("/api/v1/notifications", get(tasks::notifications))
        .route("/api/v1/tool-approvals/{id}", put(approvals::decide))
        .route(
            "/api/v1/transcriptions",
            post(voice::transcribe).layer(DefaultBodyLimit::max(voice::MAX_AUDIO_BYTES)),
        )
        .route("/api/v1/syntheses", post(voice::synthesize))
        .fallback_service(ServeDir::new(web_dir))
        .with_state(state);
    SECURITY_HEADERS
        .into_iter()
        .fold(router, |router, (name, value)| {
            router.layer(SetResponseHeaderLayer::if_not_present(
                name,
                HeaderValue::from_static(value),
            ))
        })
        .layer(TraceLayer::new_for_http())
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "voice": state.voice.is_some(),
        "cloud_model": state.cloud_model,
    }))
}
