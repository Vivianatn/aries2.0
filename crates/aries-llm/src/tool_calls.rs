//! Reconstitution des appels d'outils à partir des fragments d'une réponse streamée.

use serde::Deserialize;
use serde_json::Value;

use crate::{FunctionCall, ToolCall, ToolCallKind};

/// Fragment d'appel d'outil dans `delta.tool_calls`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ToolCallDelta {
    /// Absent chez certains fournisseurs (Gemini) qui envoient chaque appel en entier.
    index: Option<usize>,
    id: Option<String>,
    #[serde(default)]
    function: FunctionDelta,
    extra_content: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct ToolCallAccumulator {
    calls: Vec<ToolCallDelta>,
}

impl ToolCallAccumulator {
    pub(crate) fn push(&mut self, delta: ToolCallDelta) {
        let Some(index) = delta.index else {
            self.calls.push(delta);
            return;
        };
        if self.calls.len() <= index {
            self.calls.resize_with(index + 1, ToolCallDelta::default);
        }
        let call = &mut self.calls[index];
        call.id = call.id.take().or(delta.id);
        if call.function.name.as_deref().is_none_or(str::is_empty) {
            call.function.name = delta.function.name;
        }
        if let Some(fragment) = delta.function.arguments {
            call.function
                .arguments
                .get_or_insert_default()
                .push_str(&fragment);
        }
        call.extra_content = call.extra_content.take().or(delta.extra_content);
    }

    /// Appels complets ; les fragments sans nom de fonction sont ignorés.
    pub(crate) fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_iter()
            .enumerate()
            .filter_map(|(index, call)| {
                let name = call.function.name.filter(|name| !name.is_empty())?;
                Some(ToolCall {
                    id: call
                        .id
                        .filter(|id| !id.is_empty())
                        .unwrap_or_else(|| format!("call_{index}")),
                    kind: ToolCallKind::Function,
                    function: FunctionCall {
                        name,
                        arguments: call
                            .function
                            .arguments
                            .filter(|args| !args.trim().is_empty())
                            .unwrap_or_else(|| "{}".into()),
                    },
                    extra_content: call.extra_content,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn delta(value: Value) -> ToolCallDelta {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn merges_indexed_fragments() {
        let mut acc = ToolCallAccumulator::default();
        acc.push(delta(
            json!({ "index": 0, "id": "a", "function": { "name": "web_search", "arguments": "{\"q" } }),
        ));
        acc.push(delta(
            json!({ "index": 0, "function": { "arguments": "uery\":\"x\"}" } }),
        ));
        acc.push(delta(
            json!({ "index": 1, "function": { "name": "read_web_page" } }),
        ));
        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "a");
        assert_eq!(calls[0].function.arguments, r#"{"query":"x"}"#);
        assert_eq!(calls[1].id, "call_1");
        assert_eq!(calls[1].function.arguments, "{}");
    }

    #[test]
    fn keeps_unindexed_calls_separate() {
        let mut acc = ToolCallAccumulator::default();
        for name in ["a", "b"] {
            acc.push(delta(json!({
                "id": name,
                "function": { "name": name, "arguments": "{}" },
                "extra_content": { "google": { "thought_signature": "sig" } }
            })));
        }
        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].function.name, "b");
        assert!(calls[0].extra_content.is_some());
    }
}
