//! Gouvernance des outils : chaque outil déclare un niveau de risque, chaque appel est
//! audité, et les actions `danger` attendent une validation humaine.

mod approvals;
mod audit;

pub use approvals::{Approvals, PendingApproval};
pub use audit::{AuditEntry, AuditLog};

use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Lecture seule : libre.
    Read,
    /// Modifie quelque chose, de façon réversible : journalisé.
    Action,
    /// Irréversible ou sensible (envoyer, payer, supprimer) : validation humaine.
    Danger,
}

impl RiskLevel {
    pub fn needs_approval(self) -> bool {
        self == Self::Danger
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Exécuté sans validation (niveaux `read` et `action`).
    Automatic,
    Approved,
    Rejected,
    /// Pas de réponse humaine dans le délai : refusé.
    Expired,
}

impl Decision {
    pub fn allows_execution(self) -> bool {
        matches!(self, Self::Automatic | Self::Approved)
    }
}

/// Section `[governance]` de `config/aries.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GovernanceConfig {
    pub audit_path: PathBuf,
    /// Délai laissé à l'utilisateur pour valider une action `danger`.
    pub approval_timeout_secs: u64,
}

impl GovernanceConfig {
    pub fn approval_timeout(&self) -> Duration {
        Duration::from_secs(self.approval_timeout_secs)
    }
}

impl Default for GovernanceConfig {
    fn default() -> Self {
        Self {
            audit_path: "data/audit.jsonl".into(),
            approval_timeout_secs: 300,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GovernanceError {
    #[error("journal d'audit : {0}")]
    Io(#[from] std::io::Error),
    #[error("journal d'audit : {0}")]
    Json(#[from] serde_json::Error),
}
