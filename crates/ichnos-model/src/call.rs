//! Tool calls made of logged events: [`pair_calls`] joins a call's `PreToolUse` and
//! `PostToolUse` events into one [`Call`], and [`Key`] names every entry the viewer shows.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use ichnos_core::event::{Event, Status};
use serde_json::Value;

/// The viewer's identity for an entry: a tool call's id, or `event:epoch_ms:agent_id`.
///
/// Summaries, selection and expanded rows are all keyed by it, so it must stay the same when an
/// entry is rebuilt from newer data.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(String);

impl Key {
    /// The key as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A key derived from `self`, such as `turn:<key>`.
    pub(crate) fn prefixed(&self, prefix: &str) -> Self {
        Self(format!("{prefix}:{}", self.0))
    }
}

impl From<&str> for Key {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for Key {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The entry key of `event`: its tool call's id, or `event:epoch_ms:agent_id`.
#[must_use]
pub fn key_of(event: &Event) -> Key {
    match event.tool_use_id() {
        Some(id) => Key(id.to_owned()),
        None => Key(format!(
            "{}:{}:{}",
            event.name(),
            event.epoch_ms(),
            event.agent_id()
        )),
    }
}

/// A tool call's events joined into one entry, or a single other event.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    key: Key,
    /// Every event of the call, in arrival order.
    events: Vec<Arc<Event>>,
    start: i64,
    end: Option<i64>,
    status: Option<Status>,
}

impl Call {
    /// The entry key.
    #[must_use]
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// The call's events, in arrival order; the first is usually its `PreToolUse`.
    #[must_use]
    pub fn events(&self) -> &[Arc<Event>] {
        &self.events
    }

    /// The first event, which names the call's tool and agent.
    #[must_use]
    pub fn first(&self) -> &Event {
        &self.events[0]
    }

    /// The last event that finished the call, if one arrived.
    fn finish(&self) -> Option<&Event> {
        self.events
            .iter()
            .rev()
            .find(|event| event.name() != "PreToolUse")
            .map(AsRef::as_ref)
            .filter(|_| self.is_tool_call())
    }

    /// Whether this is a tool call rather than a lone event.
    #[must_use]
    pub fn is_tool_call(&self) -> bool {
        self.first().is_tool_event()
    }

    /// The hook event name of the first event.
    #[must_use]
    pub fn name(&self) -> &str {
        self.first().name()
    }

    /// The agent that made the call.
    #[must_use]
    pub fn agent_id(&self) -> &str {
        self.first().agent_id()
    }

    /// The type of agent that made the call.
    #[must_use]
    pub fn agent_type(&self) -> Option<&str> {
        self.first().agent_type()
    }

    /// The tool's name.
    #[must_use]
    pub fn tool_name(&self) -> Option<&str> {
        self.first().tool_name()
    }

    /// The call's id.
    #[must_use]
    pub fn tool_use_id(&self) -> Option<&str> {
        self.first().tool_use_id()
    }

    /// When the call started.
    #[must_use]
    pub fn start(&self) -> i64 {
        self.start
    }

    /// When the call finished; `None` while it runs.
    #[must_use]
    pub fn end(&self) -> Option<i64> {
        self.end
    }

    /// How the call stands.
    #[must_use]
    pub fn status(&self) -> Option<Status> {
        self.status
    }

    /// How long the call took, as the hook measured it.
    #[must_use]
    pub fn duration_ms(&self) -> Option<i64> {
        self.finish().and_then(Event::duration_ms)
    }

    /// The tool's input: from the first event, or from the one that finished it.
    #[must_use]
    pub fn input(&self) -> &Value {
        let first = self.first().input();
        match self.finish() {
            Some(finish) if first.is_null() => finish.input(),
            _ => first,
        }
    }

    /// The tool's response.
    #[must_use]
    pub fn response(&self) -> &Value {
        self.finish().map_or(&Value::Null, Event::response)
    }

    /// The tool's error.
    #[must_use]
    pub fn error(&self) -> &Value {
        self.finish().map_or(&Value::Null, Event::error)
    }

    /// The prompt, on a `UserPromptSubmit` entry.
    #[must_use]
    pub fn prompt(&self) -> &Value {
        self.first().prompt()
    }

    /// One of the first event's extra payload fields.
    #[must_use]
    pub fn extra(&self, name: &str) -> Option<&Value> {
        self.first().extra(name)
    }

    /// Mark a call that never finished as interrupted at `at`.
    pub(crate) fn interrupt(&mut self, at: i64) {
        self.end = Some(at);
        self.status = Some(Status::Unfinished);
    }
}

/// Join `PreToolUse`/`PostToolUse` pairs by call id into one [`Call`]; other events stand alone.
///
/// Calls come out in the order their first event arrived.
#[must_use]
pub fn pair_calls(events: &[Arc<Event>]) -> Vec<Call> {
    let mut by_id: HashMap<&str, usize> = HashMap::new();
    let mut calls: Vec<Call> = Vec::new();
    for event in events {
        let lone = Call {
            key: key_of(event),
            events: vec![Arc::clone(event)],
            start: event.epoch_ms(),
            end: Some(event.epoch_ms()),
            status: event.status(),
        };
        let Some(id) = event.tool_use_id().filter(|_| event.is_tool_event()) else {
            calls.push(lone);
            continue;
        };
        let index = *by_id.entry(id).or_insert_with(|| {
            calls.push(Call { end: None, ..lone });
            calls.len() - 1
        });
        let call = &mut calls[index];
        if !Arc::ptr_eq(&call.events[0], event) {
            call.events.push(Arc::clone(event));
        }
        if event.name() != "PreToolUse" {
            call.end = Some(event.epoch_ms());
            call.status = event.status();
        }
    }
    calls
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::*;

    /// An event from a JSON record, numbering rows in the order they're made.
    pub(crate) fn event(record: &Value) -> Arc<Event> {
        Arc::new(Event::parse(0, &record.to_string()).unwrap())
    }

    #[test]
    fn pre_and_post_join_into_one_call_and_other_events_stay_alone() {
        let events = [
            event(&json!({"event": "UserPromptSubmit", "epoch_ms": 1, "agent_id": "main"})),
            event(
                &json!({"event": "PreToolUse", "epoch_ms": 2, "tool_use_id": "t1",
                          "tool_name": "Bash", "status": "started"}),
            ),
            event(
                &json!({"event": "PostToolUse", "epoch_ms": 7, "tool_use_id": "t1",
                          "status": "ok", "response": "files"}),
            ),
            event(&json!({"event": "Stop", "epoch_ms": 9, "agent_id": "main"})),
        ];
        let calls = pair_calls(&events);
        assert_eq!(calls.len(), 3);
        let call = &calls[1];
        assert_eq!(
            (call.start(), call.end(), call.status(), call.response()),
            (2, Some(7), Some(Status::Ok), &json!("files"))
        );
        assert_eq!(call.events().len(), 2);
    }

    #[test]
    fn keys_are_the_call_id_or_event_time_and_agent() {
        let call = event(&json!({"event": "PreToolUse", "epoch_ms": 1, "tool_use_id": "t1"}));
        let stop = event(&json!({"event": "Stop", "epoch_ms": 2, "agent_id": "main"}));
        assert_eq!(key_of(&call).as_str(), "t1");
        assert_eq!(key_of(&stop).as_str(), "Stop:2:main");
    }
}
