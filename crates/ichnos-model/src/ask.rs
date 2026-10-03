//! The "Ask an agent" prompt: tells a coding agent where a session's database is,
//! how to query it safely, which row to focus on, and the user's question.

use std::path::Path;

use crate::labels::{Summaries, clock, label};
use crate::model::{NodeId, NodeKind, SessionModel};
use ichnos_core::event::Status;

/// Questions offered as one-click starters.
pub const STARTERS: [&str; 4] = [
    "Summarize what happened in this session.",
    "Why did the failed calls fail, and were they recovered?",
    "Which files were changed, and why?",
    "Where did the time go?",
];

/// How much of a row's text the focus quotes, in characters.
const FOCUS_TEXT_CHARS: usize = 200;

/// The row an agent should focus on, as text with its exact database keys.
#[must_use]
pub fn focus_of(id: NodeId, model: &SessionModel, summaries: &Summaries) -> String {
    let node = model.node(id);
    let text: String = label(node, summaries, model)
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(FOCUS_TEXT_CHARS)
        .collect();
    let start = node.start().unwrap_or_default();
    match node.kind() {
        NodeKind::Turn { prompt: None, .. } => {
            format!("the events before the first prompt (epoch_ms below {start})")
        }
        NodeKind::Turn {
            prompt: Some(prompt),
            ..
        } => {
            let ms = prompt.first().epoch_ms();
            format!(
                "the turn started by the UserPromptSubmit event with epoch_ms = {ms} ({}): \"{text}\". \
                 Its events run until the next UserPromptSubmit.",
                clock(ms)
            )
        }
        NodeKind::Model(model) => format!(
            "the model message at {} (\"{text}\"): the AssistantMessage events with \
             record->>'$.message_id' = '{}'.",
            clock(start),
            model.step().message_id().unwrap_or("unknown")
        ),
        NodeKind::Agent(agent) | NodeKind::Note(agent) => {
            let kind = agent
                .agent_type()
                .map(|kind| format!(" ({kind})"))
                .unwrap_or_default();
            format!(
                "the subagent with agent_id = '{}'{kind}: \"{text}\"",
                agent.agent_id()
            )
        }
        NodeKind::Call { call, agent } => {
            let status = match node.status() {
                Some(Status::Error) => "failed ",
                Some(Status::Started) => "running ",
                Some(Status::Unfinished) => "interrupted ",
                _ => "",
            };
            let started = agent
                .as_ref()
                .map(|agent| {
                    format!(
                        " It started the subagent with agent_id = '{}'; its own events carry that agent_id.",
                        agent.agent_id()
                    )
                })
                .unwrap_or_default();
            format!(
                "the {status}{} call with tool_use_id = '{}' by agent '{}' at {}: \"{text}\"{started}",
                call.tool_name().unwrap_or_default(),
                call.tool_use_id().unwrap_or_default(),
                call.agent_type().unwrap_or(call.agent_id()),
                clock(call.start())
            )
        }
        NodeKind::Event(call) => {
            let ms = call.first().epoch_ms();
            format!(
                "the {} event with epoch_ms = {ms} ({}): \"{text}\"",
                call.name(),
                clock(ms)
            )
        }
    }
}

/// What the ask prompt is about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AskArgs<'a> {
    /// The session id.
    pub session: &'a str,
    /// The session's database, if known.
    pub path: Option<&'a Path>,
    /// The folder the session worked in, if known.
    pub cwd: Option<&'a str>,
    /// The row to focus on, from [`focus_of`].
    pub focus: Option<&'a str>,
    /// The user's question; may be empty.
    pub question: &'a str,
}

/// The whole prompt to paste into an agent.
#[must_use]
pub fn ask_prompt(args: &AskArgs<'_>) -> String {
    let db = args.path.map_or_else(
        || format!("~/.ichnos/sessions/{}.db", args.session),
        |path| path.display().to_string(),
    );
    let worked_in = args
        .cwd
        .filter(|cwd| !cwd.is_empty())
        .map(|cwd| format!(" (worked in {cwd})"))
        .unwrap_or_default();
    let question = match args.question.trim() {
        "" => "<write your question here>",
        question => question,
    };
    let mut lines = vec![
        "Answer a question about a recorded coding-agent session, using its Ichnos log.".to_owned(),
        String::new(),
        format!("Session: {}{worked_in}", args.session),
        format!("Database: {db}"),
        String::new(),
        "How to read it:".to_owned(),
        format!(
            "- Run `sqlite3 -json \"{db}\" \"SELECT …\"`. Run SELECTs only. Don't pass -readonly: it fails on a cleanly closed database."
        ),
        "- Table events(id, epoch_ms, event, tool_use_id, record): one row per event (SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, PostToolUseFailure, SubagentStart, SubagentStop, Stop, AssistantMessage). `record` is JSON: agent_id, agent_type, tool_name, status, duration_ms, input, response, error, prompt, extra (cwd, last_assistant_message). An AssistantMessage is (a piece of) the model's text: message_id, texts, thinking, tool_use_ids, usage; pieces with the same message_id form one message.".to_owned(),
        "- A tool call is a PreToolUse plus a PostToolUse or PostToolUseFailure with the same tool_use_id.".to_owned(),
        "- Table summaries(event_id, summary): one plain-English sentence per finished call, keyed by its PreToolUse id; may be missing.".to_owned(),
        "- Nested JSON keys need the $ form: record->>'$.extra.cwd'. Fields over 8000 characters are stored as {truncated, original_length, head}.".to_owned(),
        "- A session can hold megabytes: always LIMIT rows and substr() long fields. Start with an outline (one row per UserPromptSubmit with its call and error counts), then drill in.".to_owned(),
        "- If the `ichnos` skill is installed, use its recipes.".to_owned(),
        String::new(),
    ];
    if let Some(focus) = args.focus.filter(|focus| !focus.is_empty()) {
        lines.push(format!("Focus on {focus}"));
        lines.push(String::new());
    }
    lines.push(format!("Question: {question}"));
    lines.push(String::new());
    lines.push("Answer from the rows you read, and cite event ids.".to_owned());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::call::Key;
    use crate::model::build_session;
    use crate::model::tests::{DONE, data};

    #[test]
    fn the_prompt_names_the_database_how_to_query_it_and_the_question() {
        let prompt = ask_prompt(&AskArgs {
            session: "s1",
            path: Some(Path::new("/data/sessions/s1.db")),
            cwd: Some("/code/app"),
            focus: None,
            question: "  Why did it fail? ",
        });
        let lines: Vec<&str> = prompt.lines().collect();
        assert!(lines.contains(&"Session: s1 (worked in /code/app)"));
        assert!(lines.contains(&"Database: /data/sessions/s1.db"));
        assert!(prompt.contains("sqlite3 -json \"/data/sessions/s1.db\""));
        assert!(prompt.contains("record->>'$.extra.cwd'"));
        assert!(lines.contains(&"Question: Why did it fail?"));
        assert!(!prompt.contains("Focus on"));
    }

    #[test]
    fn without_a_question_or_path_the_prompt_still_reads_sensibly() {
        let prompt = ask_prompt(&AskArgs {
            session: "s1",
            ..AskArgs::default()
        });
        let lines: Vec<&str> = prompt.lines().collect();
        assert!(lines.contains(&"Database: ~/.ichnos/sessions/s1.db"));
        assert!(lines.contains(&"Question: <write your question here>"));
    }

    #[test]
    fn a_focused_row_is_named_by_the_keys_an_agent_can_query() {
        let events = [
            json!({"event": "UserPromptSubmit", "epoch_ms": 1000, "agent_id": "main", "prompt": "fix the bug"}),
            json!({"event": "PreToolUse", "epoch_ms": 2000, "tool_use_id": "t1", "tool_name": "Bash",
                   "input": {"command": "cargo test"}, "agent_id": "main", "agent_type": "main"}),
            json!({"event": "PostToolUseFailure", "epoch_ms": 3000, "tool_use_id": "t1", "status": "error", "error": "boom"}),
        ];
        let model = build_session(&data(&events, &[]), DONE);
        let summaries = Summaries::new();
        let call = focus_of(model.find(&Key::from("t1")).unwrap(), &model, &summaries);
        assert!(call.starts_with("the failed Bash call with tool_use_id = 't1' by agent 'main'"));
        assert!(call.contains("\"cargo test\""));
        let turn = focus_of(model.turns()[0], &model, &summaries);
        assert!(turn.contains("UserPromptSubmit event with epoch_ms = 1000"));
    }
}
