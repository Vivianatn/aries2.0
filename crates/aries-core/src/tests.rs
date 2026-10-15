use std::sync::Mutex;

use aries_governance::RiskLevel;
use aries_llm::{
    ChatStream, CompletionOptions, FunctionCall, LlmError, Role, ToolCallKind, ToolSpec,
};
use aries_tools::Tool;
use async_trait::async_trait;
use futures::stream;
use serde_json::json;

use super::*;

/// LLM simulé : rejoue une réponse par tour et garde les messages reçus.
struct ScriptedLlm {
    rounds: Mutex<Vec<Vec<ChatEvent>>>,
    received: Mutex<Vec<(Vec<Message>, usize)>>,
}

impl ScriptedLlm {
    fn new(mut rounds: Vec<Vec<ChatEvent>>) -> Arc<Self> {
        rounds.reverse();
        Arc::new(Self {
            rounds: Mutex::new(rounds),
            received: Mutex::default(),
        })
    }
}

#[async_trait]
impl LlmProvider for ScriptedLlm {
    async fn chat_stream(
        &self,
        messages: Vec<Message>,
        tools: &[ToolSpec],
    ) -> Result<ChatStream, LlmError> {
        self.received.lock().unwrap().push((messages, tools.len()));
        let round = self.rounds.lock().unwrap().pop().unwrap_or_default();
        Ok(stream::iter(round.into_iter().map(Ok)).boxed())
    }

    async fn complete(&self, _: Vec<Message>, _: CompletionOptions) -> Result<String, LlmError> {
        unimplemented!()
    }
}

struct Echo(RiskLevel);

#[async_trait]
impl Tool for Echo {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".into(),
            description: "renvoie son argument".into(),
            parameters: json!({ "type": "object" }),
        }
    }

    fn risk(&self) -> RiskLevel {
        self.0
    }

    async fn call(&self, arguments: Value) -> Result<String, ToolError> {
        Ok(format!("écho : {}", arguments["text"]))
    }
}

fn echo_call() -> ChatEvent {
    ChatEvent::ToolCalls(vec![ToolCall {
        id: "c1".into(),
        kind: ToolCallKind::Function,
        function: FunctionCall {
            name: "echo".into(),
            arguments: r#"{"text":"salut"}"#.into(),
        },
        extra_content: None,
    }])
}

async fn agent(llm: Arc<ScriptedLlm>, risk: RiskLevel, name: &str) -> Arc<Agent> {
    let path = std::env::temp_dir().join(format!("aries-core-{name}-{}.jsonl", std::process::id()));
    let mut tools = ToolRegistry::default();
    tools.register(Echo(risk));
    let config = GovernanceConfig {
        audit_path: path.clone(),
        approval_timeout_secs: 5,
    };
    let audit = AuditLog::open(&path).await.unwrap();
    Arc::new(Agent::new(Some(llm.clone()), llm, tools, audit, config, 2))
}

async fn collect(mut rx: mpsc::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn calls_tool_then_answers() {
    let llm = ScriptedLlm::new(vec![
        vec![ChatEvent::Text("Je regarde. ".into()), echo_call()],
        vec![ChatEvent::Text("C'est fait.".into())],
    ]);
    let agent = agent(llm.clone(), RiskLevel::Read, "read").await;
    let (tx, rx) = mpsc::channel(32);

    let answer = agent
        .run(
            vec![Message::user("vas-y")],
            RunContext {
                conversation_id: 1,
                private: false,
                interactive: true,
            },
            &tx,
        )
        .await
        .text;
    drop(tx);
    let events = collect(rx).await;

    assert_eq!(answer, "Je regarde. C'est fait.");
    let names: Vec<_> = events.iter().map(AgentEvent::name).collect();
    assert_eq!(names, ["token", "tool_call", "tool_result", "token"]);

    let received = llm.received.lock().unwrap();
    let (second, tools) = &received[1];
    assert_eq!(*tools, 1);
    let result = second.last().unwrap();
    assert_eq!(result.role, Role::Tool);
    assert_eq!(result.tool_call_id.as_deref(), Some("c1"));
    assert_eq!(result.content, "écho : \"salut\"");
}

#[tokio::test]
async fn danger_tool_waits_for_approval() {
    let llm = ScriptedLlm::new(vec![vec![echo_call()], vec![ChatEvent::Text("ok".into())]]);
    let agent = agent(llm.clone(), RiskLevel::Danger, "danger").await;
    let (tx, mut rx) = mpsc::channel(32);

    let runner = agent.clone();
    let task = tokio::spawn(async move {
        runner
            .run(
                vec![Message::user("go")],
                RunContext {
                    conversation_id: 1,
                    private: false,
                    interactive: true,
                },
                &tx,
            )
            .await
            .text
    });

    let approval_id = loop {
        match rx.recv().await.unwrap() {
            AgentEvent::ApprovalRequired { approval_id, .. } => break approval_id,
            _ => continue,
        }
    };
    assert!(agent.approvals().resolve(approval_id, false));
    assert_eq!(task.await.unwrap(), "ok");

    let received = llm.received.lock().unwrap();
    let result = received[1].0.last().unwrap();
    assert!(result.content.contains("refusé"));
}

#[tokio::test]
async fn last_round_has_no_tools() {
    let llm = ScriptedLlm::new(vec![
        vec![echo_call()],
        vec![echo_call()],
        vec![echo_call()],
    ]);
    let agent = agent(llm.clone(), RiskLevel::Read, "rounds").await;
    let (tx, _rx) = mpsc::channel(32);

    agent
        .run(
            vec![Message::user("boucle")],
            RunContext {
                conversation_id: 1,
                private: false,
                interactive: true,
            },
            &tx,
        )
        .await;

    let tools: Vec<_> = llm
        .received
        .lock()
        .unwrap()
        .iter()
        .map(|(_, n)| *n)
        .collect();
    assert_eq!(tools, [1, 1, 0]);
}

struct Secret;

#[async_trait]
impl Tool for Secret {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".into(),
            description: "lit un mail".into(),
            parameters: json!({ "type": "object" }),
        }
    }

    fn risk(&self) -> RiskLevel {
        RiskLevel::Read
    }

    fn private(&self) -> bool {
        true
    }

    async fn call(&self, _: Value) -> Result<String, ToolError> {
        Ok("contenu confidentiel".into())
    }
}

#[tokio::test]
async fn private_result_never_reaches_cloud_llm() {
    let cloud = ScriptedLlm::new(vec![vec![echo_call()]]);
    let local = ScriptedLlm::new(vec![vec![ChatEvent::Text("résumé local".into())]]);
    let path =
        std::env::temp_dir().join(format!("aries-core-private-{}.jsonl", std::process::id()));
    let mut tools = ToolRegistry::default();
    tools.register(Secret);
    let config = GovernanceConfig {
        audit_path: path.clone(),
        approval_timeout_secs: 5,
    };
    let audit = AuditLog::open(&path).await.unwrap();
    let agent = Agent::new(Some(cloud.clone()), local.clone(), tools, audit, config, 2);
    let (tx, rx) = mpsc::channel(32);

    let reply = agent
        .run(
            vec![Message::user("lis")],
            RunContext {
                conversation_id: 1,
                private: false,
                interactive: true,
            },
            &tx,
        )
        .await;
    drop(tx);

    assert_eq!(reply.text, "résumé local");
    assert!(reply.private);
    assert!(collect(rx).await.contains(&AgentEvent::PrivateMode {}));
    assert_eq!(cloud.received.lock().unwrap().len(), 1);
    let received = local.received.lock().unwrap();
    assert!(
        received[0]
            .0
            .iter()
            .any(|m| m.content == "contenu confidentiel")
    );
}

#[tokio::test]
async fn danger_tool_is_refused_without_anyone_to_approve() {
    let llm = ScriptedLlm::new(vec![vec![echo_call()], vec![ChatEvent::Text("ok".into())]]);
    let agent = agent(llm.clone(), RiskLevel::Danger, "unattended").await;
    let (tx, rx) = mpsc::channel(32);
    let context = RunContext {
        conversation_id: 1,
        private: false,
        interactive: false,
    };

    agent.run(vec![Message::user("go")], context, &tx).await;
    drop(tx);

    let events = collect(rx).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::ApprovalRequired { .. }))
    );
    assert!(events.contains(&AgentEvent::ToolResult {
        call_id: "c1".into(),
        name: "echo".into(),
        success: false,
        decision: Decision::Unattended,
    }));
}

struct Down;

#[async_trait]
impl LlmProvider for Down {
    async fn chat_stream(&self, _: Vec<Message>, _: &[ToolSpec]) -> Result<ChatStream, LlmError> {
        Err(LlmError::QuotaExceeded(0))
    }

    async fn complete(&self, _: Vec<Message>, _: CompletionOptions) -> Result<String, LlmError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn cloud_failure_falls_back_to_local_with_local_tools_only() {
    let local = ScriptedLlm::new(vec![vec![ChatEvent::Text("réponse locale".into())]]);
    let path = std::env::temp_dir().join(format!("aries-core-down-{}.jsonl", std::process::id()));
    let mut tools = ToolRegistry::default();
    tools.register(Echo(RiskLevel::Read));
    let config = GovernanceConfig {
        audit_path: path.clone(),
        approval_timeout_secs: 5,
    };
    let audit = AuditLog::open(&path).await.unwrap();
    let agent = Agent::new(Some(Arc::new(Down)), local.clone(), tools, audit, config, 2);
    let (tx, _rx) = mpsc::channel(32);
    let context = RunContext {
        conversation_id: 1,
        private: false,
        interactive: true,
    };

    let reply = agent.run(vec![Message::user("salut")], context, &tx).await;

    assert_eq!(reply.text, "réponse locale");
    assert!(!reply.private && !reply.failed);
    // `echo` n'est pas un outil local : le modèle local ne le voit pas.
    assert_eq!(local.received.lock().unwrap()[0].1, 0);
}
