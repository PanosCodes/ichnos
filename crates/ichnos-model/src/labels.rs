//! What a row says: its chip, its text, durations and token counts.
//!
//! Pure. `summaries` is the map of entry key to summary sentence from
//! [`SessionData::summaries`](crate::session_data::SessionData::summaries).

use std::collections::HashMap;
use std::fmt::Write as _;

use serde_json::Value;

use crate::call::{Call, Key};
use crate::model::{Node, NodeKind, SessionModel, Totals};
use crate::step::Step;

/// Entry key -> summary sentence.
pub type Summaries = HashMap<Key, String>;

/// A field's text: the string itself, a clipped field's head, or its JSON.
#[must_use]
pub fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => match other.get("head").and_then(Value::as_str) {
            Some(head) if !head.is_empty() => head.to_owned(),
            _ => other.to_string(),
        },
    }
}

/// The local time of day of an epoch-millisecond moment, as `14:05:09`.
#[must_use]
pub fn clock(ms: i64) -> String {
    ichnos_core::local_time(ms, "%H:%M:%S").unwrap_or_default()
}

/// Input fields that tell the most about a call, best first.
const TELLING_FIELDS: [&str; 6] = [
    "command",
    "file_path",
    "pattern",
    "description",
    "prompt",
    "skill",
];

/// The most telling part of a tool's input: the command, path, pattern, …
#[must_use]
pub fn summarize(input: &Value) -> String {
    match input {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Object(fields) => {
            if fields.get("truncated").and_then(Value::as_bool) == Some(true) {
                return text(input);
            }
            TELLING_FIELDS
                .iter()
                .find_map(|&name| {
                    fields
                        .get(name)
                        .and_then(Value::as_str)
                        .filter(|v| !v.is_empty())
                })
                .map_or_else(|| input.to_string(), str::to_owned)
        }
        other => other.to_string(),
    }
}

/// A duration: `850ms`, `4.2s`, `3m 5s`.
#[must_use]
pub fn duration(ms: i64) -> String {
    if ms < 1000 {
        return format!("{ms}ms");
    }
    if ms < 60_000 {
        return format!("{:.1}s", tenths(ms, 1000));
    }
    let seconds = (ms + 500) / 1000;
    format!("{}m {}s", seconds / 60, seconds % 60)
}

/// A token count: `950`, `1.2k`, `34k`, `1.5M`.
#[must_use]
pub fn tokens(n: i64) -> String {
    match n {
        ..=0 => "0".to_owned(),
        1..1000 => n.to_string(),
        1000..10_000 => format!("{:.1}k", tenths(n, 1000)),
        10_000..1_000_000 => format!("{}k", (n + 500) / 1000),
        _ => format!("{:.1}M", tenths(n, 1_000_000)),
    }
}

/// `n / unit` rounded to tenths, as a float for one-decimal formatting.
#[expect(
    clippy::cast_precision_loss,
    reason = "durations and token counts are far below 2^52"
)]
fn tenths(n: i64, unit: i64) -> f64 {
    ((n * 10 + unit / 2) / unit) as f64 / 10.0
}

/// Model time, tool time and tokens of a turn or subagent.
#[must_use]
pub fn totals_line(totals: Totals) -> String {
    format!(
        "model {} · tools {} · {} out / {} in tokens",
        duration(totals.model_ms),
        duration(totals.tool_ms),
        tokens(totals.output_tokens),
        tokens(totals.input_tokens)
    )
}

/// The tools a model step called, as `Bash ×2, Read`.
#[must_use]
pub fn tool_names_of(step: &Step, model: &SessionModel) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for id in step.tool_use_ids() {
        // A call made before logging started has no node.
        let name = model
            .find(&Key::from(id.as_str()))
            .and_then(|node| model.node(node).call())
            .and_then(Call::tool_name)
            .unwrap_or("unlogged tool");
        match counts.iter_mut().find(|(known, _)| *known == name) {
            Some((_, count)) => *count += 1,
            None => counts.push((name, 1)),
        }
    }
    let mut out = String::new();
    for (i, (name, count)) in counts.into_iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(name);
        if count > 1 {
            let _ = write!(out, " ×{count}");
        }
    }
    out
}

/// How a row's chip is colored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChipClass {
    /// A turn: the user's prompt.
    Turn,
    /// A model step.
    Model,
    /// A subagent.
    Agent,
    /// A tool call or other event.
    Plain,
}

/// How a node reads in a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// The chip: `you`, `model`, the tool name, the agent type, …
    pub chip: String,
    /// How the chip is colored.
    pub chip_class: ChipClass,
    /// The row's text.
    pub text: String,
    /// Whether the text is plain language (a summary or the model's own words) rather than raw
    /// input.
    pub ai: bool,
    /// Whether the text is filler, shown dimmed.
    pub quiet: bool,
    /// A second line: a turn's outcome and totals.
    pub sub: Option<String>,
}

impl Label {
    fn new(
        chip: impl Into<String>,
        chip_class: ChipClass,
        text: impl Into<String>,
        ai: bool,
    ) -> Self {
        Self {
            chip: chip.into(),
            chip_class,
            text: text.into(),
            ai,
            quiet: false,
            sub: None,
        }
    }

    fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }
}

/// The summary of `call`'s entry, if one arrived.
fn summary<'a>(call: Option<&Call>, summaries: &'a Summaries) -> Option<&'a str> {
    summaries.get(call?.key()).map(String::as_str)
}

/// A non-empty string, or `None`.
fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// The final message on a `Stop` or `SubagentStop` entry.
fn last_message(call: Option<&Call>) -> String {
    call.and_then(|call| call.extra("last_assistant_message"))
        .map(text)
        .unwrap_or_default()
}

/// How `node` reads in a row.
#[must_use]
pub fn label(node: &Node, summaries: &Summaries, model: &SessionModel) -> Label {
    match node.kind() {
        NodeKind::Turn { prompt: None, .. } => {
            let mut label = Label::new("session", ChipClass::Turn, "Before the first prompt", true);
            label.sub = Some(totals_line(node.totals()));
            label
        }
        NodeKind::Turn {
            prompt: Some(prompt),
            stop,
        } => {
            let ai = summary(Some(prompt), summaries);
            let outcome = stop.as_ref().map_or_else(String::new, |stop| {
                summary(Some(stop), summaries)
                    .map_or_else(|| last_message(Some(stop)), str::to_owned)
            });
            let mut label = Label::new(
                "you",
                ChipClass::Turn,
                ai.map_or_else(|| text(prompt.prompt()), str::to_owned),
                ai.is_some(),
            );
            let totals = totals_line(node.totals());
            label.sub = Some(if outcome.is_empty() {
                totals
            } else {
                format!("→ {outcome}\n{totals}")
            });
            label
        }
        NodeKind::Model(model_step) => {
            let step = model_step.step();
            if !step.texts().is_empty() {
                return Label::new("model", ChipClass::Model, step.texts().join("\n"), true);
            }
            if !step.thinking().is_empty() {
                return Label::new("model", ChipClass::Model, step.thinking().join("\n"), true)
                    .quiet();
            }
            // A running step's tool calls haven't happened yet.
            if node.end().is_none() {
                return Label::new("model", ChipClass::Model, "thinking…", true).quiet();
            }
            let tools = tool_names_of(step, model);
            let text = if tools.is_empty() {
                "answered".to_owned()
            } else {
                format!("chose {tools}")
            };
            Label::new("model", ChipClass::Model, text, true).quiet()
        }
        NodeKind::Note(agent) => {
            let text = non_empty(last_message(agent.stop_event()))
                .unwrap_or_else(|| "Agent progress note".to_owned());
            Label::new("note", ChipClass::Plain, text, true)
        }
        NodeKind::Agent(agent) => {
            let text = summary(agent.stop_event(), summaries)
                .unwrap_or("Subagent started by the agent itself (no Agent call)");
            Label::new(
                agent.agent_type().unwrap_or("subagent"),
                ChipClass::Agent,
                text,
                true,
            )
        }
        NodeKind::Call {
            call,
            agent: Some(agent),
        } => {
            let ai = summary(agent.stop_event(), summaries)
                .or_else(|| summaries.get(node.key()).map(String::as_str));
            let input = call.input();
            let field = |name: &str| input.get(name).and_then(Value::as_str);
            let chip = agent
                .agent_type()
                .or_else(|| field("subagent_type"))
                .unwrap_or("subagent");
            let text = ai.map_or_else(
                || field("description").map_or_else(|| summarize(input), str::to_owned),
                str::to_owned,
            );
            Label::new(chip, ChipClass::Agent, text, ai.is_some())
        }
        NodeKind::Call { call, agent: None } | NodeKind::Event(call) => {
            let ai = summaries.get(node.key());
            let chip = call.tool_name().unwrap_or(call.name());
            let input = if call.input().is_null() {
                call.prompt()
            } else {
                call.input()
            };
            let text = match ai {
                Some(ai) => ai.clone(),
                None => non_empty(summarize(input)).unwrap_or_else(|| last_message(Some(call))),
            };
            Label::new(chip, ChipClass::Plain, text, ai.is_some())
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::build_session;
    use crate::model::tests::{DONE, data};

    #[test]
    fn durations_and_token_counts_read_naturally() {
        assert_eq!(
            [duration(850), duration(4200), duration(185_000)],
            ["850ms", "4.2s", "3m 5s"]
        );
        assert_eq!(
            [
                tokens(0),
                tokens(950),
                tokens(1234),
                tokens(34_000),
                tokens(1_500_000)
            ],
            ["0", "950", "1.2k", "34k", "1.5M"]
        );
        assert_eq!(
            summarize(&json!({"command": "ls -la", "description": "list"})),
            "ls -la"
        );
        assert_eq!(
            summarize(&json!({"truncated": true, "head": "start…"})),
            "start…"
        );
    }

    pub(crate) fn label_events() -> Vec<Value> {
        vec![
            json!({"event": "UserPromptSubmit", "epoch_ms": 1, "agent_id": "main", "prompt": "fix the bug"}),
            json!({"event": "PreToolUse", "epoch_ms": 2, "tool_use_id": "t1", "tool_name": "Bash",
                   "input": {"command": "cargo test"}, "agent_id": "main"}),
            json!({"event": "PostToolUse", "epoch_ms": 3, "tool_use_id": "t1", "status": "error", "error": "boom"}),
            json!({"event": "PreToolUse", "epoch_ms": 4, "tool_use_id": "t2", "tool_name": "Read",
                   "input": {"file_path": "/src/main.rs"}, "agent_id": "main"}),
            json!({"event": "PostToolUse", "epoch_ms": 5, "tool_use_id": "t2", "status": "ok"}),
        ]
    }

    #[test]
    fn a_summary_replaces_the_raw_input_in_a_row() {
        let model = build_session(&data(&label_events(), &[]), DONE);
        let t1 = model.node(model.find(&Key::from("t1")).unwrap());

        let raw = label(t1, &Summaries::new(), &model);
        assert_eq!(
            (raw.chip.as_str(), raw.text.as_str(), raw.ai),
            ("Bash", "cargo test", false)
        );

        let summaries =
            Summaries::from([(Key::from("t1"), "Ran the tests; they failed.".to_owned())]);
        let summarized = label(t1, &summaries, &model);
        assert_eq!(
            (summarized.text.as_str(), summarized.ai),
            ("Ran the tests; they failed.", true)
        );
    }
}
