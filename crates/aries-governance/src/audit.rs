//! Journal d'audit en JSONL, en ajout seul : une ligne par appel d'outil.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
};

use crate::{Decision, GovernanceError, RiskLevel};

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub conversation_id: Option<i64>,
    pub tool: String,
    pub risk: RiskLevel,
    pub arguments: Value,
    pub decision: Decision,
    pub success: bool,
    /// Début du résultat ou message d'erreur.
    pub outcome: String,
    pub duration_ms: u64,
}

#[derive(Serialize)]
struct AuditLine<'a> {
    timestamp: String,
    #[serde(flatten)]
    entry: &'a AuditEntry,
}

pub struct AuditLog {
    file: Mutex<File>,
}

impl AuditLog {
    pub async fn open(path: &Path) -> Result<Self, GovernanceError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).await?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    pub async fn record(&self, entry: &AuditEntry) -> Result<(), GovernanceError> {
        let mut line = serde_json::to_vec(&AuditLine {
            timestamp: jiff::Timestamp::now().to_string(),
            entry,
        })?;
        line.push(b'\n');
        let mut file = self.file.lock().await;
        file.write_all(&line).await?;
        file.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn appends_one_json_line_per_entry() {
        let path = std::env::temp_dir().join(format!("aries-audit-{}.jsonl", std::process::id()));
        let _ = fs::remove_file(&path).await;
        let log = AuditLog::open(&path).await.unwrap();
        let entry = AuditEntry {
            conversation_id: Some(1),
            tool: "web_search".into(),
            risk: RiskLevel::Read,
            arguments: json!({ "query": "météo" }),
            decision: Decision::Automatic,
            success: true,
            outcome: "ok".into(),
            duration_ms: 12,
        };
        log.record(&entry).await.unwrap();
        log.record(&entry).await.unwrap();

        let content = fs::read_to_string(&path).await.unwrap();
        let lines: Vec<Value> = content
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["tool"], "web_search");
        assert_eq!(lines[0]["risk"], "read");
        assert!(lines[0]["timestamp"].is_string());
        let _ = fs::remove_file(&path).await;
    }
}
