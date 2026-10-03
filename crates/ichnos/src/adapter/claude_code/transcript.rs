//! Read the model's side of the conversation from Claude Code's transcripts.
//!
//! Claude Code's hooks only describe tool calls. The transcript (`<project>/<session>.jsonl`, plus
//! `<project>/<session>/subagents/agent-<id>.jsonl` per subagent) also records every assistant
//! message: its text, its thinking (usually stored empty), the tools it asked for and its token
//! usage. [`new_messages`] reads the lines added since the last call and turns each assistant
//! line into one `AssistantMessage` event.
//!
//! Claude Code writes a message block by block, one line each, so one message becomes several
//! events with the same `message_id`; the viewer merges them. `agent_id` is `main` or the
//! subagent id, matching the hook events.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::tail::{self, Line};
use ichnos_core::event::ASSISTANT_MESSAGE;
use ichnos_core::event::MAIN_AGENT;
use ichnos_core::store::SessionId;
use serde_json::{Map, Value, json};

/// Token counts copied from a message's `usage`.
const USAGE_KEYS: [&str; 4] = [
    "input_tokens",
    "output_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
];

/// Byte offset read so far in each transcript file, keyed by its path.
pub type Offsets = HashMap<String, u64>;

/// `AssistantMessage` events for `session` from the lines added to the transcript at `main` and
/// its subagent transcripts since `offsets`, which are moved past them. An unreadable file is
/// left for the next call.
pub fn new_messages(main: &Path, session: &str, offsets: &mut Offsets) -> Vec<Map<String, Value>> {
    let mut files = vec![(main.to_owned(), MAIN_AGENT.to_owned())];
    let subagents_dir = main.with_extension("").join("subagents");
    if let Ok(entries) = fs::read_dir(&subagents_dir) {
        let mut subagents: Vec<(PathBuf, String)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let id = name
                    .strip_prefix("agent-")?
                    .strip_suffix(".jsonl")?
                    .to_owned();
                Some((entry.path(), id))
            })
            .collect();
        subagents.sort();
        files.extend(subagents);
    }

    let mut events = Vec::new();
    for (path, agent_id) in files {
        let key = path.to_string_lossy().into_owned();
        let offset = offsets.get(&key).copied().unwrap_or(0);
        for Line { text, end } in tail::new_lines(&path, offset).unwrap_or_default() {
            offsets.insert(key.clone(), end);
            events.extend(message_event(&text, session, &agent_id));
        }
    }
    events
}

/// The `AssistantMessage` event for one transcript line; `None` for lines that aren't an
/// assistant message of `agent_id`.
fn message_event(line: &str, session: &str, agent_id: &str) -> Option<Map<String, Value>> {
    let entry: Value = serde_json::from_str(line).ok()?;
    let is_sidechain = entry
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if entry.get("type").and_then(Value::as_str) != Some("assistant")
        || (is_sidechain && agent_id == MAIN_AGENT)
    {
        return None;
    }
    let message = entry.get("message")?;
    let content = message.get("content")?.as_array()?;
    let ms = epoch_ms(entry.get("timestamp")?.as_str()?)?;
    let message_id = message
        .get("id")
        .or_else(|| entry.get("uuid"))
        .and_then(Value::as_str);

    let usage: Map<String, Value> = USAGE_KEYS
        .iter()
        .filter_map(|&name| {
            let value = message.get("usage")?.get(name)?;
            value.is_i64().then(|| (name.to_owned(), value.clone()))
        })
        .collect();
    let (mut texts, mut thinking, mut tool_use_ids) = (Vec::new(), Vec::new(), Vec::new());
    for block in content {
        let field = |name: &str| {
            block
                .get(name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        match block.get("type").and_then(Value::as_str) {
            Some("text") => texts.extend(field("text")),
            Some("thinking") => thinking.extend(field("thinking")),
            Some("tool_use") => tool_use_ids.extend(field("id")),
            _ => {}
        }
    }
    let event = json!({
        "event": ASSISTANT_MESSAGE,
        "session_id": session,
        "epoch_ms": ms,
        "agent_id": agent_id,
        "message_id": message_id,
        "model": message.get("model").cloned().unwrap_or(Value::Null),
        "first_ms": ms,
        "last_ms": ms,
        "usage": usage,
        "texts": texts,
        "thinking": thinking,
        "tool_use_ids": tool_use_ids,
    });
    match event {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// The main transcript of `session` under `~/.claude/projects/<project>/`, for sessions whose
/// events carry no `transcript_path`.
pub fn find_in_projects(session: &SessionId) -> Option<PathBuf> {
    let file_name = format!("{session}.jsonl");
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))?;
    fs::read_dir(config_dir.join("projects"))
        .ok()?
        .flatten()
        .map(|project| project.path().join(&file_name))
        .find(|candidate| candidate.exists())
}

/// Milliseconds since the epoch for an RFC 3339 timestamp such as `2026-10-01T21:55:00.123Z`.
fn epoch_ms(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|time| time.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    #[test]
    fn each_assistant_line_is_one_event_and_sidechains_are_skipped_on_main() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("s1.jsonl");
        let mut lines = String::new();
        for (ts, block) in [
            (
                "2026-10-01T10:00:00.000Z",
                r#"{"type":"text","text":" hello "}"#,
            ),
            (
                "2026-10-01T10:00:02.000Z",
                r#"{"type":"tool_use","id":"t1"}"#,
            ),
        ] {
            writeln!(
                lines,
                r#"{{"type":"assistant","timestamp":"{ts}","message":{{"id":"m1","model":"opus","usage":{{"output_tokens":5}},"content":[{block}]}}}}"#
            )
            .unwrap();
        }
        lines.push_str(r#"{"type":"assistant","isSidechain":true,"timestamp":"2026-10-01T10:00:03Z","message":{"id":"m2","content":[]}}"#);
        lines.push('\n');
        lines.push_str(r#"{"type":"user","timestamp":"2026-10-01T10:00:04Z"}"#);
        lines.push('\n');
        fs::write(&transcript, lines).unwrap();

        let mut offsets = Offsets::new();
        let events = new_messages(&transcript, "s1", &mut offsets);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["event"], json!("AssistantMessage"));
        assert_eq!(events[0]["session_id"], json!("s1"));
        assert_eq!(events[0]["message_id"], json!("m1"));
        assert_eq!(events[0]["texts"], json!(["hello"]));
        assert_eq!(events[0]["usage"], json!({"output_tokens": 5}));
        assert_eq!(events[1]["tool_use_ids"], json!(["t1"]));
        assert_eq!(
            events[1]["epoch_ms"].as_i64().unwrap() - events[0]["epoch_ms"].as_i64().unwrap(),
            2000
        );
        assert!(
            new_messages(&transcript, "s1", &mut offsets).is_empty(),
            "nothing new on the second read"
        );
    }

    #[test]
    fn subagent_transcripts_are_read_under_their_agent_id() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("s1.jsonl");
        fs::write(&transcript, "").unwrap();
        let subagents = dir.path().join("s1/subagents");
        fs::create_dir_all(&subagents).unwrap();
        fs::write(
            subagents.join("agent-a1.jsonl"),
            r#"{"type":"assistant","isSidechain":true,"timestamp":"2026-10-01T10:00:00Z","message":{"id":"m9","content":[{"type":"text","text":"sub"}]}}"#.to_owned() + "\n",
        )
        .unwrap();

        let events = new_messages(&transcript, "s1", &mut Offsets::new());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["agent_id"], json!("a1"));
    }
}
