//! Agent d'ArIès : le LLM répond ou appelle des outils, dont les résultats lui sont
//! renvoyés, jusqu'à la réponse finale. Chaque appel passe par la gouvernance.

use std::{sync::Arc, time::Instant};

use aries_governance::{Approvals, AuditEntry, AuditLog, Decision, GovernanceConfig};
use aries_llm::{ChatEvent, ChatStream, LlmError, LlmProvider, Message, Role, ToolCall, ToolSpec};
use aries_memory::{Memory, MemoryError};
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
    /// Message enregistré (`reply` seulement, si la réponse n'est pas vide).
    pub message_id: Option<i64>,
    /// Interrompue par une erreur (LLM indisponible…), signalée par un événement `Error`.
    pub failed: bool,
}

/// Cadre d'une réponse.
#[derive(Debug, Clone, Copy)]
pub struct RunContext {
    pub conversation_id: i64,
    /// Données privées : modèle local uniquement.
    pub private: bool,
    /// Quelqu'un peut valider les actions `danger` ; sinon (tâche de fond) elles sont refusées.
    pub interactive: bool,
}

pub struct Agent {
    /// Modèle cloud du chat, s'il est configuré ; en cas d'échec, bascule sur `local`.
    cloud: Option<Arc<dyn LlmProvider>>,
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
        cloud: Option<Arc<dyn LlmProvider>>,
        local: Arc<dyn LlmProvider>,
        tools: ToolRegistry,
        audit: AuditLog,
        config: GovernanceConfig,
        max_rounds: usize,
    ) -> Self {
        Self {
            cloud,
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

    /// Tour complet dans une conversation : enregistre `content`, répond avec l'historique
    /// et la mémoire, marque la conversation privée si besoin, enregistre la réponse.
    pub async fn reply(
        &self,
        memory: &Memory,
        persona: &str,
        conversation_id: i64,
        content: &str,
        interactive: bool,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Result<Reply, MemoryError> {
        let private = memory
            .conversation(conversation_id)
            .await?
            .is_some_and(|conversation| conversation.private);
        memory
            .append_message(conversation_id, Role::User, content)
            .await?;
        // Souvenirs privés seulement pour le modèle local (conversation déjà privée).
        let prompt = memory
            .build_prompt(conversation_id, persona, private)
            .await?;
        let context = RunContext {
            conversation_id,
            private,
            interactive,
        };
        let mut reply = self.run(prompt, context, events).await;

        // Avant d'enregistrer la réponse : la consolidation doit la voir comme privée.
        if reply.private && !private {
            memory.mark_private(conversation_id).await?;
        }
        if !reply.text.trim().is_empty() {
            reply.message_id = Some(
                memory
                    .append_message(conversation_id, Role::Assistant, &reply.text)
                    .await?,
            );
        }
        Ok(reply)
    }

    /// Répond à `messages` en relayant les événements (erreurs comprises). Une
    /// conversation privée n'est traitée que par le modèle local ; elle le devient
    /// dès qu'un outil privé répond.
    pub async fn run(
        &self,
        mut messages: Vec<Message>,
        context: RunContext,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Reply {
        let (specs, local_specs) = (self.tools.specs(), self.tools.local_specs());
        let mut reply = Reply {
            private: context.private,
            ..Reply::default()
        };
        // Modèle local : données privées, pas de cloud, ou cloud tombé pendant ce tour.
        let mut local = context.private || self.cloud.is_none();

        for round in 0..=self.max_rounds {
            local |= reply.private;
            // Dernier tour : plus d'outils, le modèle doit conclure.
            let last = round == self.max_rounds;
            let mut stream = match self
                .open_stream(&messages, &mut local, last, &specs, &local_specs)
                .await
            {
                Ok(stream) => stream,
                Err(err) => {
                    tracing::error!(%err, "LLM indisponible");
                    reply.failed = true;
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
                        reply.failed = true;
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
            if calls.is_empty() || last {
                return reply;
            }

            messages.push(Message::assistant_tool_calls(text, calls.clone()));
            for call in &calls {
                let output = self.execute(call, context, events).await;
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

    /// Cloud d'abord (tous les outils) ; s'il échoue avant de répondre, le reste du tour
    /// passe au modèle local, avec seulement les outils prévus pour son contexte court.
    async fn open_stream(
        &self,
        messages: &[Message],
        local: &mut bool,
        last: bool,
        specs: &[ToolSpec],
        local_specs: &[ToolSpec],
    ) -> Result<ChatStream, LlmError> {
        let offered = |specs| if last { &[][..] } else { specs };
        if let Some(cloud) = self.cloud.as_ref().filter(|_| !*local) {
            match cloud.chat_stream(messages.to_vec(), offered(specs)).await {
                Ok(stream) => return Ok(stream),
                Err(err) => {
                    tracing::warn!(%err, "LLM cloud indisponible, bascule sur le modèle local");
                    *local = true;
                }
            }
        }
        self.local
            .chat_stream(messages.to_vec(), offered(local_specs))
            .await
    }

    /// Exécute un appel sous gouvernance ; le résultat (ou l'erreur) est pour le LLM.
    async fn execute(
        &self,
        call: &ToolCall,
        context: RunContext,
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

        let decision = if risk.needs_approval() && !context.interactive {
            Decision::Unattended
        } else if risk.needs_approval() {
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
            Decision::Unattended => Err(ToolError::Refused(
                "action sensible impossible en tâche de fond (personne pour la valider) : \
                 propose-la à l'utilisateur dans ta réponse"
                    .into(),
            )),
        };
        let success = result.is_ok();
        let output = result.unwrap_or_else(|err| format!("Erreur : {err}."));

        let entry = AuditEntry {
            conversation_id: Some(context.conversation_id),
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
