//! Logged hook events as typed values.
//!
//! The hook stores each event as a JSON record (see `hook.rs` in the CLI). [`Event::parse`] reads
//! one back, keeping the original text for searching and copying.

use std::fmt;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::store::Row;

/// The event name of one piece of a model message.
pub const ASSISTANT_MESSAGE: &str = "AssistantMessage";

/// The agent id of the main agent, as the hook records it.
pub const MAIN_AGENT: &str = "main";

/// How a tool call, or an entry made of calls, stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Running: started and not finished yet.
    Started,
    /// Finished successfully.
    Ok,
    /// Finished with an error.
    Error,
    /// Never finished: its turn or subagent ended first, so it was interrupted.
    Unfinished,
}

impl Status {
    /// The status as the hook writes it: `started`, `ok`, `error` or `unfinished`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Unfinished => "unfinished",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One logged hook event.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Event {
    /// The database row id; it orders events and names them in summaries.
    #[serde(skip)]
    row_id: i64,
    epoch_ms: i64,
    /// The hook event name.
    #[serde(rename = "event")]
    name: String,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_type: Option<String>,
    #[serde(default)]
    tool_use_id: Option<String>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    status: Option<Status>,
    #[serde(default)]
    duration_ms: Option<i64>,
    #[serde(default)]
    input: Value,
    #[serde(default)]
    response: Value,
    #[serde(default)]
    error: Value,
    #[serde(default)]
    prompt: Value,
    #[serde(default)]
    extra: Map<String, Value>,
    /// The record exactly as stored, for searching and "copy as JSON".
    #[serde(skip)]
    json: String,
}

impl Event {
    /// The event stored as `json` in row `row_id`.
    ///
    /// # Errors
    ///
    /// Returns the [`serde_json::Error`] when `json` isn't an event record.
    pub fn parse(row_id: i64, json: &str) -> Result<Self, serde_json::Error> {
        let mut event: Self = serde_json::from_str(json)?;
        event.row_id = row_id;
        json.clone_into(&mut event.json);
        Ok(event)
    }

    /// The event stored in `row`.
    ///
    /// # Errors
    ///
    /// Returns the [`serde_json::Error`] when the row doesn't hold an event record.
    pub fn from_row(row: &Row) -> Result<Self, serde_json::Error> {
        Self::parse(row.id, &row.json)
    }

    /// The database row id.
    #[must_use]
    pub fn row_id(&self) -> i64 {
        self.row_id
    }

    /// When the hook received the event, in epoch milliseconds.
    #[must_use]
    pub fn epoch_ms(&self) -> i64 {
        self.epoch_ms
    }

    /// The hook event name: `PreToolUse`, `Stop`, `SubagentStart`, …
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The agent that sent the event: [`MAIN_AGENT`] or a subagent id.
    #[must_use]
    pub fn agent_id(&self) -> &str {
        self.agent_id.as_deref().unwrap_or(MAIN_AGENT)
    }

    /// Whether the main agent sent the event.
    #[must_use]
    pub fn is_main(&self) -> bool {
        self.agent_id() == MAIN_AGENT
    }

    /// The agent's type, such as `Explore`; `main` for the main agent.
    #[must_use]
    pub fn agent_type(&self) -> Option<&str> {
        self.agent_type.as_deref()
    }

    /// The tool call this event belongs to.
    #[must_use]
    pub fn tool_use_id(&self) -> Option<&str> {
        self.tool_use_id.as_deref()
    }

    /// The tool called, such as `Bash`.
    #[must_use]
    pub fn tool_name(&self) -> Option<&str> {
        self.tool_name.as_deref()
    }

    /// The call's status as the hook recorded it.
    #[must_use]
    pub fn status(&self) -> Option<Status> {
        self.status
    }

    /// How long the call took, on its `PostToolUse` event.
    #[must_use]
    pub fn duration_ms(&self) -> Option<i64> {
        self.duration_ms
    }

    /// The tool's input.
    #[must_use]
    pub fn input(&self) -> &Value {
        &self.input
    }

    /// The tool's response.
    #[must_use]
    pub fn response(&self) -> &Value {
        &self.response
    }

    /// The tool's error.
    #[must_use]
    pub fn error(&self) -> &Value {
        &self.error
    }

    /// The prompt, on `UserPromptSubmit`.
    #[must_use]
    pub fn prompt(&self) -> &Value {
        &self.prompt
    }

    /// One of the `extra` fields: `cwd`, `last_assistant_message`, `source`, …
    #[must_use]
    pub fn extra(&self, name: &str) -> Option<&Value> {
        self.extra.get(name)
    }

    /// The record exactly as stored.
    #[must_use]
    pub fn json(&self) -> &str {
        &self.json
    }

    /// Whether this is one half of a tool call: a `…ToolUse…` event with a call id.
    #[must_use]
    pub fn is_tool_event(&self) -> bool {
        self.tool_use_id.is_some() && self.name.contains("ToolUse")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_original_record_is_kept() {
        let text = r#"{"event":"Stop","epoch_ms":2}"#;
        let event = Event::parse(4, text).unwrap();
        assert_eq!((event.row_id(), event.json()), (4, text));
    }
}
