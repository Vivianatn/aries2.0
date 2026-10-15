mod config;
mod error;
mod routes;

use std::{env, fs, sync::Arc};

use anyhow::Context;
use aries_core::Agent;
use aries_governance::AuditLog;
use aries_llm::{FallbackProvider, LlmProvider, OpenAiCompatProvider};
use aries_memory::{ActivityGauge, Memory};
use aries_tools::ToolRegistry;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::{
    config::{CloudLlmConfig, Config},
    routes::AppState,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::load()?;
    let persona = fs::read_to_string(&config.assistant.persona_path)
        .with_context(|| format!("lecture de {}", config.assistant.persona_path.display()))?;
    let local_llm: Arc<dyn LlmProvider> = Arc::new(OpenAiCompatProvider::new(
        &config.llm.local.base_url,
        &config.llm.local.model,
        None,
    )?);
    let chat_llm = chat_provider(config.llm.cloud.as_ref(), &local_llm)?;

    let memory = Arc::new(
        Memory::connect(config.memory.clone())
            .await
            .context("connexion à la mémoire (SQLite, Qdrant)")?,
    );
    let activity = ActivityGauge::default();
    memory
        .clone()
        .spawn_consolidation(local_llm, activity.clone());

    let tools = ToolRegistry::with_web_tools(&config.tools)?;
    let audit = AuditLog::open(&config.governance.audit_path)
        .await
        .with_context(|| {
            format!(
                "ouverture du journal d'audit {}",
                config.governance.audit_path.display()
            )
        })?;
    let agent = Arc::new(Agent::new(
        chat_llm,
        tools,
        audit,
        config.governance.clone(),
        config.tools.max_rounds,
    ));

    let state = AppState {
        agent,
        memory,
        activity,
        persona: persona.into(),
    };
    let app = routes::router(state, &config.server.web_dir);

    let listener = TcpListener::bind(config.server.bind).await?;
    tracing::info!(addr = %config.server.bind, "ArIès est en ligne");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Cloud en priorité avec bascule sur le local si sa clé est définie, sinon local seul.
fn chat_provider(
    cloud: Option<&CloudLlmConfig>,
    local: &Arc<dyn LlmProvider>,
) -> anyhow::Result<Arc<dyn LlmProvider>> {
    let Some(cloud) = cloud else {
        tracing::info!("chat : modèle local uniquement");
        return Ok(local.clone());
    };
    let Some(api_key) = env::var(&cloud.api_key_env)
        .ok()
        .filter(|key| !key.is_empty())
    else {
        tracing::info!(variable = %cloud.api_key_env, "chat : clé cloud absente, modèle local uniquement");
        return Ok(local.clone());
    };
    let provider = OpenAiCompatProvider::new(&cloud.base_url, &cloud.model, Some(api_key))?
        .with_reasoning_effort(cloud.reasoning_effort.clone());
    tracing::info!(model = %cloud.model, "chat : cloud, avec bascule sur le modèle local");
    Ok(Arc::new(FallbackProvider::new(
        Arc::new(provider),
        local.clone(),
    )))
}

/// Ctrl+C en local, SIGTERM sous Docker.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("arrêt d'ArIès");
}
