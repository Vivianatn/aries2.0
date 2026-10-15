//! Tâches planifiées dans leur propre base SQLite (`data/tasks.db`).

use std::path::Path;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::ProactiveError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum TaskKind {
    /// Message envoyé tel quel.
    Reminder,
    /// Consigne exécutée par l'agent ; sa réponse est envoyée.
    Agent,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub kind: TaskKind,
    pub content: String,
    pub cron: Option<String>,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub conversation_id: Option<i64>,
    pub active: bool,
}

pub struct NewTask<'a> {
    pub title: &'a str,
    pub kind: TaskKind,
    pub content: &'a str,
    pub cron: Option<&'a str>,
    pub next_run_at: Timestamp,
}

/// Format fixe (UTC, à la seconde) : les dates se comparent comme des chaînes.
pub fn utc(timestamp: Timestamp) -> String {
    timestamp.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

const COLUMNS: &str =
    "id, title, kind, content, cron, next_run_at, last_run_at, conversation_id, active";

#[derive(Clone)]
pub struct TaskStore {
    pool: SqlitePool,
}

impl TaskStore {
    pub async fn open(path: &Path) -> Result<Self, ProactiveError> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn insert(&self, task: NewTask<'_>) -> Result<i64, ProactiveError> {
        Ok(sqlx::query_scalar(
            "INSERT INTO tasks (title, kind, content, cron, next_run_at)
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(task.title)
        .bind(task.kind)
        .bind(task.content)
        .bind(task.cron)
        .bind(utc(task.next_run_at))
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn active(&self) -> Result<Vec<Task>, ProactiveError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM tasks WHERE active = 1 ORDER BY next_run_at"
        ))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn due(&self, now: Timestamp) -> Result<Vec<Task>, ProactiveError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM tasks WHERE active = 1 AND next_run_at <= ? ORDER BY next_run_at"
        ))
        .bind(utc(now))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn next_run_at(&self) -> Result<Option<Timestamp>, ProactiveError> {
        let next: Option<String> =
            sqlx::query_scalar("SELECT MIN(next_run_at) FROM tasks WHERE active = 1")
                .fetch_one(&self.pool)
                .await?;
        Ok(next.and_then(|next| next.parse().ok()))
    }

    /// `false` si la tâche n'existait pas ou était déjà terminée.
    pub async fn cancel(&self, id: i64) -> Result<bool, ProactiveError> {
        let result = sqlx::query("UPDATE tasks SET active = 0 WHERE id = ? AND active = 1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_conversation(
        &self,
        id: i64,
        conversation_id: i64,
    ) -> Result<(), ProactiveError> {
        sqlx::query("UPDATE tasks SET conversation_id = ? WHERE id = ?")
            .bind(conversation_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Enregistre une exécution ; `next` = prochaine échéance, ou `None` pour terminer.
    pub async fn record_run(
        &self,
        id: i64,
        started_at: Timestamp,
        success: bool,
        output: &str,
        next: Option<Timestamp>,
    ) -> Result<(), ProactiveError> {
        let finished_at = utc(Timestamp::now());
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO task_runs (task_id, started_at, finished_at, success, output)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(utc(started_at))
        .bind(&finished_at)
        .bind(success)
        .bind(output)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE tasks SET last_run_at = ?, next_run_at = ?, active = ? WHERE id = ?")
            .bind(&finished_at)
            .bind(next.map(utc))
            .bind(next.is_some())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use jiff::ToSpan;

    use super::*;

    #[tokio::test]
    async fn due_tasks_run_once_then_finish() {
        let path = std::env::temp_dir().join(format!("aries-tasks-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = TaskStore::open(&path).await.unwrap();
        let now = Timestamp::now();
        let soon = store
            .insert(NewTask {
                title: "appeler maman",
                kind: TaskKind::Reminder,
                content: "Appelle maman",
                cron: None,
                next_run_at: now - 1.minute(),
            })
            .await
            .unwrap();
        store
            .insert(NewTask {
                title: "plus tard",
                kind: TaskKind::Agent,
                content: "météo",
                cron: Some("0 9 * * *"),
                next_run_at: now + 1.hour(),
            })
            .await
            .unwrap();

        let due = store.due(now).await.unwrap();
        assert_eq!(due.iter().map(|t| t.id).collect::<Vec<_>>(), [soon]);
        assert_eq!(due[0].kind, TaskKind::Reminder);

        store
            .record_run(soon, now, true, "envoyé", None)
            .await
            .unwrap();
        assert!(store.due(now).await.unwrap().is_empty());
        assert_eq!(store.active().await.unwrap().len(), 1);
        assert_eq!(
            store.next_run_at().await.unwrap().map(utc),
            Some(utc(now + 1.hour()))
        );
        let _ = std::fs::remove_file(&path);
    }
}
