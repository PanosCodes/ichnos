//! How nodes look in rows and the inspector: tone, icon, badge, status text and the sections
//! of the Call tab. Pure; the views turn tones into colors.

use ichnos_core::event::Status;
use ichnos_model::call::Call;
use ichnos_model::labels::{Summaries, text, tool_names_of, totals_line};
use ichnos_model::model::{Node, NodeKind, SessionModel};
use serde_json::Value;

/// A row's color family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Finished successfully.
    Ok,
    /// Failed.
    Err,
    /// Running.
    Run,
    /// A subagent.
    Agent,
    /// A finished model step.
    Model,
    /// A lone event, a note, or an interrupted call.
    Event,
}

/// How a node looks in a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    /// Color family of the badge and bar.
    pub tone: Tone,
    /// The status icon.
    pub icon: &'static str,
    /// Color family of the icon.
    pub icon_tone: Tone,
    /// The badge text: the tool, `agent`, `model`, …
    pub badge: String,
}

fn tool(call: Option<&Call>) -> String {
    call.and_then(Call::tool_name)
        .unwrap_or_default()
        .to_owned()
}

/// How `node` looks in a row.
#[must_use]
pub fn look(node: &Node) -> Look {
    let running = node.status() == Some(Status::Started);
    let look = |tone, icon, icon_tone, badge: String| Look {
        tone,
        icon,
        icon_tone,
        badge,
    };
    match node.kind() {
        NodeKind::Model(_) => {
            let tone = if running { Tone::Run } else { Tone::Model };
            look(
                tone,
                if running { "●" } else { "" },
                Tone::Run,
                "model".into(),
            )
        }
        NodeKind::Note(_) => look(Tone::Event, "·", Tone::Event, "note".into()),
        NodeKind::Event(call) => look(Tone::Event, "·", Tone::Event, call.name().to_owned()),
        NodeKind::Turn { .. } => {
            let tone = if node.errors() > 0 {
                Tone::Err
            } else if node.running() {
                Tone::Run
            } else {
                Tone::Ok
            };
            look(tone, "", tone, "turn".into())
        }
        NodeKind::Call { .. } | NodeKind::Agent(_) => {
            let agent = node.is_subagent();
            let badge = if agent {
                "agent".to_owned()
            } else {
                tool(node.call())
            };
            match node.status() {
                Some(Status::Error) => look(Tone::Err, "✕", Tone::Err, badge),
                Some(Status::Started) => look(Tone::Run, "●", Tone::Run, badge),
                Some(Status::Unfinished) => look(Tone::Event, "?", Tone::Event, tool(node.call())),
                _ if agent => look(Tone::Agent, "✓", Tone::Agent, badge),
                _ => look(Tone::Ok, "✓", Tone::Ok, badge),
            }
        }
    }
}

/// What the inspector calls a node's status.
#[must_use]
pub fn status_text(node: &Node) -> String {
    match node.kind() {
        NodeKind::Turn { .. } => {
            if node.running() {
                "running".into()
            } else if node.errors() > 0 {
                plural(node.errors(), "error")
            } else {
                "done".into()
            }
        }
        NodeKind::Model(_) if node.status() == Some(Status::Started) => "thinking".into(),
        NodeKind::Model(_) => "model step".into(),
        NodeKind::Note(_) => "progress note".into(),
        NodeKind::Event(call) => call.name().to_owned(),
        NodeKind::Call { .. } | NodeKind::Agent(_) => match node.status() {
            Some(Status::Error) => {
                let exit = node
                    .call()
                    .and_then(|call| call.response().get("exit_code"))
                    .filter(|code| !code.is_null())
                    .map(|code| format!(" · exit {code}"))
                    .unwrap_or_default();
                format!("failed{exit}")
            }
            Some(Status::Started) => "running".into(),
            Some(Status::Unfinished) => "interrupted".into(),
            Some(Status::Ok) => "ok".into(),
            None => String::new(),
        },
    }
}

/// `n thing` or `n things`.
#[must_use]
pub fn plural(n: usize, thing: &str) -> String {
    if n == 1 {
        format!("1 {thing}")
    } else {
        format!("{n} {thing}s")
    }
}

/// The output tokens a row shows: a model step's, or a subagent's total.
#[must_use]
pub fn row_tokens(node: &Node) -> i64 {
    match node.kind() {
        NodeKind::Model(model) => model.step().output_tokens(),
        _ if node.is_subagent() => node.totals().output_tokens,
        _ => 0,
    }
}

/// A tool input as the inspector shows it: the command when there is one, else JSON.
#[must_use]
pub fn input_text(input: &Value) -> String {
    match input {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        _ if input.get("truncated").and_then(Value::as_bool) == Some(true) => text(input),
        _ => match input.get("command").and_then(Value::as_str) {
            Some(command) => command.to_owned(),
            None => pretty(input),
        },
    }
}

/// A tool response as text: a shell's stdout and stderr, a clipped field's head, else JSON.
#[must_use]
pub fn output_text(response: &Value) -> String {
    match response {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        _ if response.get("truncated").and_then(Value::as_bool) == Some(true) => text(response),
        _ => match response.get("stdout").and_then(Value::as_str) {
            Some(stdout) => {
                let stderr = response
                    .get("stderr")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                [stdout, stderr]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            None => pretty(response),
        },
    }
}

/// `value` as indented JSON.
#[must_use]
pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// How a section's body is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionTone {
    /// Prose: a prompt or reply.
    Prose,
    /// Code or data, in mono.
    Code,
    /// An error, in mono and red.
    Err,
}

/// One titled block of the Call tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The heading.
    pub title: &'static str,
    /// The text.
    pub body: String,
    /// How the body is shown.
    pub tone: SectionTone,
}

/// The sections the Call tab shows for `node`: input and output, prompt and reply, …
#[must_use]
pub fn sections(node: &Node, model: &SessionModel, _summaries: &Summaries) -> Vec<Section> {
    let mut out = Vec::new();
    let mut add = |title, body: String, tone| {
        if !body.is_empty() {
            out.push(Section { title, body, tone });
        }
    };
    let last_message = |call: Option<&Call>| {
        call.and_then(|c| c.extra("last_assistant_message"))
            .map(text)
            .unwrap_or_default()
    };
    match node.kind() {
        NodeKind::Turn { prompt, stop } => {
            add(
                "Prompt",
                prompt
                    .as_ref()
                    .map(|p| text(p.prompt()))
                    .unwrap_or_default(),
                SectionTone::Prose,
            );
            add(
                "Final reply",
                last_message(stop.as_ref()),
                SectionTone::Prose,
            );
            add("Time", totals_line(node.totals()), SectionTone::Code);
        }
        NodeKind::Model(model_step) => {
            let step = model_step.step();
            add("Text", step.texts().join("\n\n"), SectionTone::Prose);
            add("Thinking", step.thinking().join("\n\n"), SectionTone::Prose);
            let tools = tool_names_of(step, model);
            add(
                "Tools called",
                if tools.is_empty() {
                    "none".into()
                } else {
                    tools
                },
                SectionTone::Code,
            );
            let usage = step.to_json().get("usage").map(pretty).unwrap_or_default();
            add("Tokens", usage, SectionTone::Code);
            let name = step
                .to_json()
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned);
            add("Model", name.unwrap_or_default(), SectionTone::Code);
        }
        NodeKind::Agent(agent) | NodeKind::Note(agent) => {
            add(
                "Returned",
                last_message(agent.stop_event()),
                SectionTone::Prose,
            );
            add("Time", totals_line(node.totals()), SectionTone::Code);
        }
        NodeKind::Call {
            call,
            agent: Some(agent),
        } => {
            let prompt = call.input().get("prompt").map(text).unwrap_or_default();
            add("Prompt", prompt, SectionTone::Prose);
            add(
                "Returned",
                last_message(agent.stop_event()),
                SectionTone::Prose,
            );
            add("Time", totals_line(node.totals()), SectionTone::Code);
        }
        NodeKind::Call { call, agent: None } | NodeKind::Event(call) => {
            if call.tool_use_id().is_some() {
                add("Input", input_text(call.input()), SectionTone::Code);
            } else if call.prompt().is_null() {
                add("Reply", last_message(Some(call)), SectionTone::Prose);
            } else {
                add("Prompt", text(call.prompt()), SectionTone::Prose);
            }
            let has_error = !call.error().is_null();
            if has_error {
                add("Error", text(call.error()), SectionTone::Err);
            }
            let output_tone = if node.status() == Some(Status::Error) && !has_error {
                SectionTone::Err
            } else {
                SectionTone::Code
            };
            add("Output", output_text(call.response()), output_tone);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn shell_output_shows_stdout_then_stderr() {
        assert_eq!(
            output_text(&json!({"stdout": "ok", "stderr": "warn"})),
            "ok\nwarn"
        );
        assert_eq!(
            output_text(&json!({"stdout": "", "stderr": "boom"})),
            "boom"
        );
        assert_eq!(
            output_text(&json!({"truncated": true, "head": "start"})),
            "start"
        );
    }

    #[test]
    fn a_command_input_shows_just_the_command() {
        assert_eq!(
            input_text(&json!({"command": "ls", "description": "list"})),
            "ls"
        );
        assert_eq!(
            input_text(&json!({"file_path": "/a"})),
            "{\n  \"file_path\": \"/a\"\n}"
        );
    }

    #[test]
    fn counts_read_naturally() {
        assert_eq!(plural(1, "error"), "1 error");
        assert_eq!(plural(3, "error"), "3 errors");
    }
}
