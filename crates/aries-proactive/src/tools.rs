//! Outils du LLM pour gérer les tâches : planifier, lister, annuler.

use std::fmt::Write;

use aries_governance::RiskLevel;
use aries_llm::ToolSpec;
use aries_tools::{Tool, ToolError};
use async_trait::async_trait;
use jiff::{Timestamp, Zoned, civil, tz::TimeZone};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{
    next_occurrence,
    store::{NewTask, TaskKind, TaskStore},
};

fn parse<T: DeserializeOwned>(arguments: Value) -> Result<T, ToolError> {
    serde_json::from_value(arguments).map_err(|err| ToolError::InvalidArguments(err.to_string()))
}

fn failed(err: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(err.to_string())
}

/// Ex. « jeudi 16/10/2026 à 09:00 » (fuseau du serveur).
pub fn local_time(timestamp: Timestamp) -> String {
    timestamp
        .to_zoned(TimeZone::system())
        .strftime("%d/%m/%Y à %H:%M")
        .to_string()
}

/// Date ISO avec fuseau, ou date locale sans fuseau (heure du serveur).
fn parse_at(at: &str) -> Result<Timestamp, ToolError> {
    at.parse::<Timestamp>()
        .or_else(|_| {
            at.parse::<civil::DateTime>()
                .and_then(|local| local.to_zoned(TimeZone::system()))
                .map(|zoned| zoned.timestamp())
        })
        .map_err(|err| ToolError::InvalidArguments(format!("date « {at} » invalide : {err}")))
}

pub struct ScheduleTask(pub TaskStore);

#[derive(Deserialize)]
struct ScheduleArgs {
    title: String,
    kind: TaskKind,
    content: String,
    at: Option<String>,
    cron: Option<String>,
}

#[async_trait]
impl Tool for ScheduleTask {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "schedule_task".into(),
            description: "Planifie un rappel ou une tâche de fond, une fois (`at`) ou de façon \
                récurrente (`cron`). Le résultat est envoyé à l'utilisateur sur Telegram. \
                kind=reminder : `content` est le message envoyé tel quel (« Appelle le \
                dentiste »). kind=agent : `content` est une consigne que tu exécuteras seule à \
                l'échéance, avec tes outils (« Cherche le prix du billet Paris-Lyon du 3/11 et \
                préviens si moins de 40 € ») ; tu n'enverras un message que s'il y a quelque \
                chose à signaler."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Titre court" },
                    "kind": { "type": "string", "enum": ["reminder", "agent"] },
                    "content": { "type": "string" },
                    "at": {
                        "type": "string",
                        "description": "Échéance unique, heure locale, ex. 2026-10-16T09:00"
                    },
                    "cron": {
                        "type": "string",
                        "description": "Récurrence cron à 5 champs, heure locale, ex. « 0 8 * * 1-5 » (8h en semaine)"
                    }
                },
                "required": ["title", "kind", "content"]
            }),
        }
    }

    fn risk(&self) -> RiskLevel {
        RiskLevel::Action
    }

    fn local(&self) -> bool {
        true
    }

    async fn call(&self, arguments: Value) -> Result<String, ToolError> {
        let args: ScheduleArgs = parse(arguments)?;
        let cron = args
            .cron
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty());
        let next_run_at = match (args.at.as_deref(), cron) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(ToolError::InvalidArguments(
                    "donner soit `at`, soit `cron`".into(),
                ));
            }
            (Some(at), None) => {
                let at = parse_at(at)?;
                if at <= Timestamp::now() {
                    return Err(ToolError::InvalidArguments(
                        "cette date est déjà passée".into(),
                    ));
                }
                at
            }
            (None, Some(cron)) => next_occurrence(cron, &Zoned::now())
                .map_err(|err| ToolError::InvalidArguments(format!("cron invalide : {err}")))?,
        };
        let id = self
            .0
            .insert(NewTask {
                title: args.title.trim(),
                kind: args.kind,
                content: args.content.trim(),
                cron,
                next_run_at,
            })
            .await
            .map_err(failed)?;
        Ok(format!(
            "Tâche n°{id} planifiée, prochaine exécution le {}.",
            local_time(next_run_at)
        ))
    }
}

pub struct ListTasks(pub TaskStore);

#[async_trait]
impl Tool for ListTasks {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_tasks".into(),
            description: "Liste les rappels et tâches de fond actifs.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    fn risk(&self) -> RiskLevel {
        RiskLevel::Read
    }

    fn local(&self) -> bool {
        true
    }

    async fn call(&self, _: Value) -> Result<String, ToolError> {
        let tasks = self.0.active().await.map_err(failed)?;
        if tasks.is_empty() {
            return Ok("Aucune tâche planifiée.".into());
        }
        let mut out = String::new();
        for task in tasks {
            let next = task
                .next_run_at
                .as_deref()
                .and_then(|next| next.parse().ok())
                .map_or_else(|| "?".into(), local_time);
            let repeat = task
                .cron
                .map_or_else(String::new, |cron| format!(", cron « {cron} »"));
            let _ = writeln!(
                out,
                "n°{} « {} » ({:?}{repeat}) : prochaine le {next}. {}",
                task.id, task.title, task.kind, task.content
            );
        }
        Ok(out)
    }
}

pub struct CancelTask(pub TaskStore);

#[derive(Deserialize)]
struct CancelArgs {
    id: i64,
}

#[async_trait]
impl Tool for CancelTask {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "cancel_task".into(),
            description: "Annule un rappel ou une tâche de fond (numéro donné par list_tasks)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "integer" } },
                "required": ["id"]
            }),
        }
    }

    fn risk(&self) -> RiskLevel {
        RiskLevel::Action
    }

    fn local(&self) -> bool {
        true
    }

    async fn call(&self, arguments: Value) -> Result<String, ToolError> {
        let CancelArgs { id } = parse(arguments)?;
        if self.0.cancel(id).await.map_err(failed)? {
            Ok(format!("Tâche n°{id} annulée."))
        } else {
            Err(ToolError::Failed(format!("pas de tâche active n°{id}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_and_offset_dates() {
        let local = parse_at("2026-10-16T09:00").unwrap();
        assert_eq!(local, parse_at("2026-10-16T09:00:00").unwrap());
        let utc = parse_at("2026-10-16T07:00:00Z").unwrap();
        assert_eq!(utc.to_string(), "2026-10-16T07:00:00Z");
        assert!(parse_at("demain").is_err());
    }
}
