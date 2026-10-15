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
    Arc::new(Agent::new(llm, tools, audit, config, 2))
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

    let answer = agent.run(vec![Message::user("vas-y")], 1, &tx).await;
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
    let task = tokio::spawn(async move { runner.run(vec![Message::user("go")], 1, &tx).await });

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

    agent.run(vec![Message::user("boucle")], 1, &tx).await;

    let tools: Vec<_> = llm
        .received
        .lock()
        .unwrap()
        .iter()
        .map(|(_, n)| *n)
        .collect();
    assert_eq!(tools, [1, 1, 0]);
}
