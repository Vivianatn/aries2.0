//! Tâches de fond : rappels et consignes planifiées (une fois ou en cron), persistantes,
//! exécutées sans surveillance (les actions `danger` y sont refusées) et notifiées.

mod store;
mod tools;

pub use store::{Notification, Task, TaskKind, TaskStore};
pub use tools::{CancelTask, ListTasks, ScheduleTask, local_time};

use std::{path::PathBuf, sync::Arc, time::Duration};

use aries_core::{Agent, Reply};
use aries_memory::{ActivityGauge, Memory};
use aries_tools::ToolRegistry;
use async_trait::async_trait;
use croner::Cron;
use jiff::{Timestamp, Zoned};
use serde::Deserialize;
use tokio::sync::mpsc;

/// Le planificateur se réveille au moins toutes les 30 s (nouvelles tâches comprises).
const MAX_SLEEP: Duration = Duration::from_secs(30);
/// Réponse de l'agent quand rien ne mérite une notification.
const NOTHING: &str = "RIEN";
const NOTIFICATION_MAX_CHARS: usize = 3_500;

#[derive(Debug, thiserror::Error)]
pub enum ProactiveError {
    #[error("base des tâches : {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration des tâches : {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("fichier : {0}")]
    Io(#[from] std::io::Error),
    #[error("mémoire : {0}")]
    Memory(#[from] aries_memory::MemoryError),
    #[error("échec de l'agent : {0}")]
    Failed(String),
}

/// Section `[proactive]` de `config/aries.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProactiveConfig {
    pub db_path: PathBuf,
}

impl Default for ProactiveConfig {
    fn default() -> Self {
        Self {
            db_path: "data/tasks.db".into(),
        }
    }
}

/// Canal qui prévient l'utilisateur (Telegram…).
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, text: &str) -> Result<(), String>;
}

/// Sans canal configuré : la notification reste dans les journaux (et la conversation).
pub struct LogNotifier;

#[async_trait]
impl Notifier for LogNotifier {
    async fn notify(&self, text: &str) -> Result<(), String> {
        tracing::info!(%text, "notification (aucun canal configuré)");
        Ok(())
    }
}

/// Prochaine occurrence strictement après `after`, dans son fuseau.
pub fn next_occurrence(cron: &str, after: &Zoned) -> Result<Timestamp, String> {
    let cron: Cron = cron.parse().map_err(|err| format!("{err}"))?;
    cron.find_next_occurrence(after, false)
        .map(|next| next.timestamp())
        .map_err(|err| format!("{err}"))
}

/// Ajoute les outils `schedule_task`, `list_tasks` et `cancel_task`.
pub fn register_tools(registry: &mut ToolRegistry, store: &TaskStore) {
    registry.register(ScheduleTask(store.clone()));
    registry.register(ListTasks(store.clone()));
    registry.register(CancelTask(store.clone()));
}

pub struct Scheduler {
    pub store: TaskStore,
    pub agent: Arc<Agent>,
    pub memory: Arc<Memory>,
    pub persona: Arc<str>,
    pub notifier: Arc<dyn Notifier>,
    pub activity: ActivityGauge,
}

impl Scheduler {
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                if let Err(err) = self.run_due().await {
                    tracing::error!(%err, "planificateur");
                }
                let wait = match self.store.next_run_at().await {
                    Ok(Some(next)) => Duration::try_from(next.duration_since(Timestamp::now()))
                        .unwrap_or_default()
                        .min(MAX_SLEEP),
                    _ => MAX_SLEEP,
                };
                tokio::time::sleep(wait.max(Duration::from_millis(500))).await;
            }
        })
    }

    /// Une échéance manquée (serveur arrêté) est exécutée une fois, puis la suivante est
    /// calculée à partir de maintenant.
    async fn run_due(&self) -> Result<(), ProactiveError> {
        for task in self.store.due(Timestamp::now()).await? {
            let started_at = Timestamp::now();
            tracing::info!(task = task.id, title = %task.title, "tâche planifiée");
            let (success, output) = match self.execute(&task).await {
                Ok(output) => (true, output),
                Err(err) => (false, err.to_string()),
            };
            let next = task.cron.as_deref().and_then(|cron| {
                next_occurrence(cron, &Zoned::now())
                    .inspect_err(
                        |err| tracing::error!(task = task.id, %err, "cron invalide, tâche arrêtée"),
                    )
                    .ok()
            });
            self.store
                .record_run(task.id, started_at, success, &output, next)
                .await?;
        }
        Ok(())
    }

    async fn execute(&self, task: &Task) -> Result<String, ProactiveError> {
        let message = match task.kind {
            TaskKind::Reminder => Some(format!("⏰ {}", task.content)),
            TaskKind::Agent => {
                let reply = self.run_agent(task).await?;
                if reply.failed {
                    let message = format!(
                        "⚠️ La tâche « {} » n'a pas pu s'exécuter (LLM indisponible). Nouvel essai à la prochaine échéance.",
                        task.title
                    );
                    if let Err(err) = self.notifier.notify(&message).await {
                        tracing::error!(task = task.id, %err, "notification non envoyée");
                    }
                    return Err(ProactiveError::Failed(reply.text));
                }
                let answer = reply.text;
                (answer.trim() != NOTHING && !answer.trim().is_empty())
                    .then(|| format!("🔔 {}\n\n{answer}", task.title))
            }
        };
        let Some(message) = message else {
            return Ok(NOTHING.into());
        };
        let message: String = message.chars().take(NOTIFICATION_MAX_CHARS).collect();
        if let Err(err) = self.notifier.notify(&message).await {
            tracing::error!(task = task.id, %err, "notification non envoyée");
        }
        Ok(message)
    }

    /// Consigne exécutée dans la conversation de la tâche (visible dans l'interface).
    async fn run_agent(&self, task: &Task) -> Result<Reply, ProactiveError> {
        // La conversation a pu être effacée depuis l'historique : on en recrée une.
        let existing = match task.conversation_id {
            Some(id) => self.memory.conversation(id).await?.map(|c| c.id),
            None => None,
        };
        let conversation_id = match existing {
            Some(id) => id,
            None => {
                let id = self.memory.create_conversation().await?.id;
                self.store.set_conversation(task.id, id).await?;
                id
            }
        };
        let prompt = format!(
            "[Tâche planifiée n°{} « {} », exécutée sans l'utilisateur]\n{}\n\n\
             Réponds en quelques phrases avec ce qui mérite d'être signalé. Si rien ne \
             mérite de l'être, réponds uniquement : {NOTHING}",
            task.id, task.title, task.content
        );
        // Personne n'écoute les événements : ils sont abandonnés.
        let (events, _) = mpsc::channel(1);
        let _busy = self.activity.begin();
        let reply = self
            .agent
            .reply(
                &self.memory,
                &self.persona,
                conversation_id,
                &prompt,
                false,
                &events,
            )
            .await?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    #[test]
    fn next_occurrence_follows_local_time() {
        let paris = date(2026, 10, 15)
            .at(10, 30, 0, 0)
            .in_tz("Europe/Paris")
            .unwrap();
        let next = next_occurrence("0 9 * * *", &paris).unwrap();
        assert_eq!(
            next.to_zoned(paris.time_zone().clone()).to_string(),
            "2026-10-16T09:00:00+02:00[Europe/Paris]"
        );
        // Passage à l'heure d'hiver le 25/10 : toujours 9h locale.
        let after = date(2026, 10, 25)
            .at(8, 0, 0, 0)
            .in_tz("Europe/Paris")
            .unwrap();
        let next = next_occurrence("0 9 * * *", &after).unwrap();
        assert_eq!(next.to_string(), "2026-10-25T08:00:00Z");
        assert!(next_occurrence("pas un cron", &paris).is_err());
    }
}
