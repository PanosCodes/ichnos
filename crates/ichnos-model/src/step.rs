//! Model messages: the agent's text between tool calls, built from `AssistantMessage` events.
//!
//! An adapter may log one message in pieces (a block at a time, as it streams) or whole. Every
//! piece with the same `agent_id` and `message_id` is folded into one [`Step`] by [`Step::merge`].

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use ichnos_core::event::MAIN_AGENT;

/// One assistant message, merged from its `AssistantMessage` events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    #[serde(default)]
    key: String,
    #[serde(default = "main_agent")]
    agent_id: String,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    model: Value,
    #[serde(default)]
    first_ms: i64,
    #[serde(default)]
    last_ms: i64,
    #[serde(default)]
    usage: Map<String, Value>,
    #[serde(default)]
    texts: Vec<String>,
    #[serde(default)]
    thinking: Vec<String>,
    #[serde(default)]
    tool_use_ids: Vec<String>,
}

fn main_agent() -> String {
    MAIN_AGENT.to_owned()
}

/// The fields of an `AssistantMessage` record that a [`Step`] doesn't name itself.
#[derive(Debug, Deserialize)]
struct Times {
    #[serde(default)]
    epoch_ms: i64,
}

impl Step {
    /// The step for one stored `AssistantMessage` record. `first_ms` and `last_ms` default to the
    /// record's `epoch_ms`.
    ///
    /// # Errors
    ///
    /// Returns the [`serde_json::Error`] when `json` isn't an `AssistantMessage` record.
    pub fn from_record(json: &str) -> Result<Self, serde_json::Error> {
        let mut step: Self = serde_json::from_str(json)?;
        let Times { epoch_ms } = serde_json::from_str(json)?;
        if step.first_ms == 0 {
            step.first_ms = epoch_ms;
        }
        if step.last_ms == 0 {
            step.last_ms = step.first_ms;
        }
        step.key = format!(
            "model:{}:{}",
            step.agent_id,
            step.message_id.as_deref().unwrap_or("None")
        );
        Ok(step)
    }

    /// Fold a later piece of the same message into this one: blocks are appended, the time span
    /// widens, and the newest non-empty usage and model win.
    pub fn merge(&mut self, piece: Self) {
        self.first_ms = self.first_ms.min(piece.first_ms);
        self.last_ms = self.last_ms.max(piece.last_ms);
        if !piece.usage.is_empty() {
            self.usage = piece.usage;
        }
        if !piece.model.is_null() {
            self.model = piece.model;
        }
        self.texts.extend(piece.texts);
        self.thinking.extend(piece.thinking);
        self.tool_use_ids.extend(piece.tool_use_ids);
    }

    /// The step's identity, `model:<agent_id>:<message_id>`; every piece of a message shares it.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The agent that wrote the message: `main` or a subagent id.
    #[must_use]
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// The API message id.
    #[must_use]
    pub fn message_id(&self) -> Option<&str> {
        self.message_id.as_deref()
    }

    /// When the message's first block was written, in epoch milliseconds.
    #[must_use]
    pub fn first_ms(&self) -> i64 {
        self.first_ms
    }

    /// When the message's latest block was written, in epoch milliseconds.
    #[must_use]
    pub fn last_ms(&self) -> i64 {
        self.last_ms
    }

    /// One of the message's token counts, such as `output_tokens`; 0 when absent.
    #[must_use]
    pub fn usage(&self, name: &str) -> i64 {
        self.usage.get(name).and_then(Value::as_i64).unwrap_or(0)
    }

    /// Tokens the model wrote.
    #[must_use]
    pub fn output_tokens(&self) -> i64 {
        self.usage("output_tokens")
    }

    /// Tokens the model read: fresh input plus cache reads and writes.
    #[must_use]
    pub fn input_tokens(&self) -> i64 {
        self.usage("input_tokens")
            + self.usage("cache_read_input_tokens")
            + self.usage("cache_creation_input_tokens")
    }

    /// The message's text blocks.
    #[must_use]
    pub fn texts(&self) -> &[String] {
        &self.texts
    }

    /// The message's thinking blocks; often empty.
    #[must_use]
    pub fn thinking(&self) -> &[String] {
        &self.thinking
    }

    /// The tool calls the message asked for.
    #[must_use]
    pub fn tool_use_ids(&self) -> &[String] {
        &self.tool_use_ids
    }

    /// The message as JSON, for "copy as JSON".
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn step(record: &Value) -> Step {
        Step::from_record(&record.to_string()).unwrap()
    }

    #[test]
    fn a_record_becomes_a_step_keyed_by_agent_and_message() {
        let step = step(&json!({
            "event": "AssistantMessage", "epoch_ms": 5, "agent_id": "a1", "message_id": "m1",
            "texts": ["hi"], "usage": {"output_tokens": 3}
        }));
        assert_eq!(step.key(), "model:a1:m1");
        assert_eq!((step.first_ms(), step.last_ms()), (5, 5));
        assert_eq!(step.output_tokens(), 3);
    }

    #[test]
    fn pieces_of_one_message_merge_in_order() {
        let mut first =
            step(&json!({"epoch_ms": 1, "message_id": "m1", "texts": ["a"], "model": "opus"}));
        first.merge(step(&json!({
            "epoch_ms": 3, "message_id": "m1", "texts": ["b"], "tool_use_ids": ["t1"],
            "usage": {"output_tokens": 9}
        })));
        assert_eq!(first.key(), "model:main:m1");
        assert_eq!(first.texts(), ["a", "b"]);
        assert_eq!(first.tool_use_ids(), ["t1"]);
        assert_eq!((first.first_ms(), first.last_ms()), (1, 3));
        assert_eq!(first.output_tokens(), 9);
        assert_eq!(first.to_json()["model"], "opus");
    }
}
