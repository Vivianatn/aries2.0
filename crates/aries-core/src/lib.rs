//! Agent d'ArIès : le LLM répond ou appelle des outils, dont les résultats lui sont
//! renvoyés, jusqu'à la réponse finale. Chaque appel passe par la gouvernance.

use std::{sync::Arc, time::Instant};

use aries_governance::{Approvals, AuditEntry, AuditLog, Decision, GovernanceConfig};
use aries_llm::{ChatEvent, LlmProvider, Message, ToolCall};
use aries_tools::{ToolError, ToolRegistry};
use futures::StreamExt;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;

const AUDIT_OUTCOME_MAX_CHARS: usize = 300;

/// Ce qui se passe pendant une réponse, pour l'interface.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AgentEvent {
    Token {
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        arguments: Value,
    },
    /// Action `danger` : attend `Agent::approvals().resolve(approval_id, …)`.
    ApprovalRequired {
        approval_id: u64,
        call_id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        call_id: String,
        name: String,
        success: bool,
        decision: Decision,
    },
    Error {
        message: String,
    },
    /// Un outil privé a répondu : la suite est traitée par le modèle local.
    PrivateMode {},
}

impl AgentEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Token { .. } => "token",
            Self::ToolCall { .. } => "tool_call",
            Self::ApprovalRequired { .. } => "approval_required",
            Self::ToolResult { .. } => "tool_result",
            Self::Error { .. } => "error",
            Self::PrivateMode {} => "private_mode",
        }
    }
}

/// Réponse produite, même partielle en cas d'erreur.
#[derive(Debug, Default, PartialEq)]
pub struct Reply {
    pub text: String,
    /// La conversation contient désormais des données privées.
    pub private: bool,
}

pub struct Agent {
    llm: Arc<dyn LlmProvider>,
    /// Modèle local : seul autorisé à voir des données privées.
    local: Arc<dyn LlmProvider>,
    tools: ToolRegistry,
    audit: AuditLog,
    approvals: Approvals,
    config: GovernanceConfig,
    max_rounds: usize,
}

impl Agent {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        local: Arc<dyn LlmProvider>,
        tools: ToolRegistry,
        audit: AuditLog,
        config: GovernanceConfig,
        max_rounds: usize,
    ) -> Self {
        Self {
            llm,
            local,
            tools,
            audit,
            approvals: Approvals::default(),
            config,
            max_rounds,
        }
    }

    pub fn approvals(&self) -> &Approvals {
        &self.approvals
    }

    /// Répond à `messages` en relayant les événements (erreurs comprises). Une
    /// conversation `private` n'est traitée que par le modèle local ; elle le devient
    /// dès qu'un outil privé répond.
    pub async fn run(
        &self,
        mut messages: Vec<Message>,
        conversation_id: i64,
        private: bool,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Reply {
        let (specs, local_specs) = (self.tools.specs(), self.tools.local_specs());
        let mut reply = Reply {
            text: String::new(),
            private,
        };

        for round in 0..=self.max_rounds {
            let (llm, specs) = if reply.private {
                (&self.local, &local_specs)
            } else {
                (&self.llm, &specs)
            };
            // Dernier tour : plus d'outils, le modèle doit conclure.
            let tools = if round < self.max_rounds {
                &specs[..]
            } else {
                &[]
            };
            let mut stream = match llm.chat_stream(messages.clone(), tools).await {
                Ok(stream) => stream,
                Err(err) => {
                    tracing::error!(%err, "LLM indisponible");
                    emit(
                        events,
                        AgentEvent::Error {
                            message: err.to_string(),
                        },
                    )
                    .await;
                    return reply;
                }
            };

            let mut text = String::new();
            let mut calls = Vec::new();
            while let Some(event) = stream.next().await {
                match event {
                    Ok(ChatEvent::Text(fragment)) => {
                        text.push_str(&fragment);
                        emit(events, AgentEvent::Token { text: fragment }).await;
                    }
                    Ok(ChatEvent::ToolCalls(requested)) => calls = requested,
                    Err(err) => {
                        tracing::warn!(%err, "flux LLM interrompu");
                        emit(
                            events,
                            AgentEvent::Error {
                                message: err.to_string(),
                            },
                        )
                        .await;
                        reply.text.push_str(&text);
                        return reply;
                    }
                }
            }
            reply.text.push_str(&text);
            if calls.is_empty() || tools.is_empty() {
                return reply;
            }

            messages.push(Message::assistant_tool_calls(text, calls.clone()));
            for call in &calls {
                let output = self.execute(call, conversation_id, events).await;
                messages.push(Message::tool_result(&call.id, output));
                let private_result = self
                    .tools
                    .get(&call.function.name)
                    .is_some_and(|tool| tool.private());
                if private_result && !reply.private {
                    reply.private = true;
                    emit(events, AgentEvent::PrivateMode {}).await;
                }
            }
        }
        reply
    }

    /// Exécute un appel sous gouvernance ; le résultat (ou l'erreur) est pour le LLM.
    async fn execute(
        &self,
        call: &ToolCall,
        conversation_id: i64,
        events: &mpsc::Sender<AgentEvent>,
    ) -> String {
        let name = &call.function.name;
        let Some(tool) = self.tools.get(name) else {
            tracing::warn!(%name, "outil inconnu demandé par le LLM");
            return format!("Erreur : l'outil « {name} » n'existe pas.");
        };
        let arguments: Value = match serde_json::from_str(&call.function.arguments) {
            Ok(arguments) => arguments,
            Err(err) => return format!("Erreur : arguments JSON invalides ({err})."),
        };
        let risk = tool.risk();
        emit(
            events,
            AgentEvent::ToolCall {
                call_id: call.id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            },
        )
        .await;

        let decision = if risk.needs_approval() {
            let pending = self.approvals.request();
            emit(
                events,
                AgentEvent::ApprovalRequired {
                    approval_id: pending.id,
                    call_id: call.id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                },
            )
            .await;
            pending.wait(self.config.approval_timeout()).await
        } else {
            Decision::Automatic
        };

        let started = Instant::now();
        let result = match decision {
            Decision::Automatic | Decision::Approved => tool.call(arguments.clone()).await,
            Decision::Rejected => Err(ToolError::Refused(
                "l'utilisateur a refusé cette action".into(),
            )),
            Decision::Expired => Err(ToolError::Refused(
                "pas de validation à temps, action annulée".into(),
            )),
        };
        let success = result.is_ok();
        let output = result.unwrap_or_else(|err| format!("Erreur : {err}."));

        let entry = AuditEntry {
            conversation_id: Some(conversation_id),
            tool: name.clone(),
            risk,
            arguments,
            decision,
            success,
            outcome: output.chars().take(AUDIT_OUTCOME_MAX_CHARS).collect(),
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        };
        if let Err(err) = self.audit.record(&entry).await {
            tracing::error!(%err, tool = %name, "appel d'outil non audité");
        }
        emit(
            events,
            AgentEvent::ToolResult {
                call_id: call.id.clone(),
                name: name.clone(),
                success,
                decision,
            },
        )
        .await;
        output
    }
}

/// Le client peut s'être déconnecté : la réponse continue quand même.
async fn emit(events: &mpsc::Sender<AgentEvent>, event: AgentEvent) {
    let _ = events.send(event).await;
}

#[cfg(test)]
mod tests;
