mod config;
mod error;
mod routes;

use std::{env, fs, sync::Arc};

use anyhow::Context;
use aries_core::Agent;
use aries_governance::AuditLog;
use aries_llm::{DailyQuota, LlmProvider, OpenAiCompatProvider};
use aries_memory::{ActivityGauge, Memory};
use aries_proactive::{LogNotifier, Scheduler, TaskStore};
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
    let cloud_llm = cloud_provider(config.llm.cloud.as_ref())?;

    let memory = Arc::new(
        Memory::connect(config.memory.clone())
            .await
            .context("connexion à la mémoire (SQLite, Qdrant)")?,
    );
    let activity = ActivityGauge::default();
    memory
        .clone()
        .spawn_consolidation(local_llm.clone(), activity.clone());

    let tasks = TaskStore::open(&config.proactive.db_path)
        .await
        .context("ouverture de la base des tâches")?;
    let mut tools = ToolRegistry::with_web_tools(&config.tools)?;
    aries_proactive::register_tools(&mut tools, &tasks);
    tools.connect_mcp(&config.tools.mcp).await;
    let audit = AuditLog::open(&config.governance.audit_path)
        .await
        .with_context(|| {
            format!(
                "ouverture du journal d'audit {}",
                config.governance.audit_path.display()
            )
        })?;
    let agent = Arc::new(Agent::new(
        cloud_llm,
        local_llm,
        tools,
        audit,
        config.governance.clone(),
        config.tools.max_rounds,
    ));

    let persona: Arc<str> = persona.into();
    Scheduler {
        store: tasks,
        agent: agent.clone(),
        memory: memory.clone(),
        persona: persona.clone(),
        notifier: Arc::new(LogNotifier),
        activity: activity.clone(),
    }
    .spawn();

    let state = AppState {
        agent,
        memory,
        activity,
        persona,
    };
    let app = routes::router(state, &config.server.web_dir);

    let listener = TcpListener::bind(config.server.bind).await?;
    tracing::info!(addr = %config.server.bind, "ArIès est en ligne");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Modèle cloud du chat si sa clé est définie (l'agent bascule sur le local s'il échoue).
fn cloud_provider(cloud: Option<&CloudLlmConfig>) -> anyhow::Result<Option<Arc<dyn LlmProvider>>> {
    let Some(cloud) = cloud else {
        tracing::info!("chat : modèle local uniquement");
        return Ok(None);
    };
    let Some(api_key) = env::var(&cloud.api_key_env)
        .ok()
        .filter(|key| !key.is_empty())
    else {
        tracing::info!(variable = %cloud.api_key_env, "chat : clé cloud absente, modèle local uniquement");
        return Ok(None);
    };
    let mut provider: Arc<dyn LlmProvider> = Arc::new(
        OpenAiCompatProvider::new(&cloud.base_url, &cloud.model, Some(api_key))?
            .with_reasoning_effort(cloud.reasoning_effort.clone()),
    );
    if let Some(limit) = cloud.daily_request_limit {
        provider = Arc::new(DailyQuota::new(provider, limit));
    }
    tracing::info!(model = %cloud.model, limit = ?cloud.daily_request_limit, "chat : cloud, avec bascule sur le modèle local");
    Ok(Some(provider))
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
