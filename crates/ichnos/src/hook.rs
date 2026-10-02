//! The Claude Code hook: turn one hook payload into one event row in its session's database.
//!
//! Wired for `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `SubagentStart`, `SubagentStop`,
//! `UserPromptSubmit`, `SessionStart` and `Stop`. Subagent tool calls carry `agent_id`/`agent_type`,
//! so the main agent and each subagent form separate lanes. A `PostToolUse` gets its `duration_ms`
//! from the stored `PreToolUse` with the same `tool_use_id`.

use std::io::{self, Read};
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Map, Value, json};
use thiserror::Error;

use ichnos_core::store::{NewEvent, SessionId, Store, StoreError};

/// Fields longer than this many characters are stored as `{truncated, original_length, head}`.
const MAX_FIELD: usize = 8000;

/// Payload keys copied into `extra` when present.
const EXTRA_KEYS: [&str; 7] = [
    "agent_transcript_path",
    "last_assistant_message",
    "source",
    "stop_hook_active",
    "is_interrupt",
    "permission_mode",
    "cwd",
];

/// Why a hook event could not be logged.
#[derive(Debug, Error)]
pub enum HookError {
    #[error("could not read the hook payload from stdin")]
    ReadPayload(#[source] io::Error),

    #[error("the hook payload is not a JSON object")]
    ParsePayload(#[source] serde_json::Error),

    #[error("could not store the event")]
    Store(#[from] StoreError),

    #[error("could not serialize the log record")]
    Serialize(#[source] serde_json::Error),
}

/// One logged event, stored as JSON; field order is the order written.
#[derive(Debug, Serialize)]
struct Record {
    ts: String,
    epoch_ms: i64,
    event: String,
    session_id: Value,
    agent_id: Value,
    agent_type: Value,
    tool_use_id: Value,
    tool_name: Value,
    status: Option<&'static str>,
    duration_ms: Option<i64>,
    input: Value,
    response: Value,
    error: Value,
    prompt: Value,
    extra: Map<String, Value>,
}

/// Read one hook payload from `input` and store its record in the session's database inside the
/// sessions folder `dir` (`<dir>/<session-id>.db`).
///
/// # Errors
///
/// Returns a [`HookError`] when stdin can't be read or parsed, the session id can't be a file name,
/// or the database can't be written.
pub fn run(dir: &Path, mut input: impl Read) -> Result<(), HookError> {
    let mut raw = String::new();
    input
        .read_to_string(&mut raw)
        .map_err(HookError::ReadPayload)?;
    let payload: Map<String, Value> = if raw.trim().is_empty() {
        Map::new()
    } else {
        serde_json::from_str(&raw).map_err(HookError::ParsePayload)?
    };
    let mut record = build_record(&payload, Utc::now());
    let session: SessionId = text(&payload, "session_id")
        .unwrap_or("unknown-session")
        .parse()?;
    let store = Store::open(&session.db_path(dir))?;
    let tool_use_id = text(&payload, "tool_use_id");

    match record.event.as_str() {
        "PreToolUse" => record.status = Some("started"),
        "PostToolUse" | "PostToolUseFailure" => {
            record.status = Some(if record.event == "PostToolUseFailure" {
                "error"
            } else {
                "ok"
            });
            if let Some(id) = tool_use_id {
                record.duration_ms = store.start_ms(id)?.map(|started| record.epoch_ms - started);
            }
        }
        _ => {}
    }

    let json = serde_json::to_string(&record).map_err(HookError::Serialize)?;
    store.add_event(&NewEvent {
        epoch_ms: record.epoch_ms,
        event: &record.event,
        tool_use_id,
        json: &json,
    })?;
    Ok(())
}

/// The record for `payload` received at `now`, before status and duration are filled in.
fn build_record(payload: &Map<String, Value>, now: DateTime<Utc>) -> Record {
    let agent_id = text(payload, "agent_id");
    let agent_type = match text(payload, "agent_type") {
        Some(kind) => json!(kind),
        None if agent_id.is_none() => json!("main"),
        None => Value::Null,
    };
    let mut extra: Map<String, Value> = EXTRA_KEYS
        .iter()
        .filter_map(|&key| payload.get(key).map(|value| (key.to_owned(), clip(value))))
        .collect();
    // The conversation transcript holds the agent's text between tool calls; a viewer tails it.
    // Recorded on the lifecycle events only, to keep the per-tool lines small.
    if let (Some(path), None) = (payload.get("transcript_path"), text(payload, "tool_use_id")) {
        extra.insert("transcript_path".to_owned(), path.clone());
    }
    let field = |key: &str| payload.get(key).map_or(Value::Null, clip);

    Record {
        ts: now.to_rfc3339_opts(SecondsFormat::Millis, false),
        epoch_ms: now.timestamp_millis(),
        event: text(payload, "hook_event_name")
            .unwrap_or("Unknown")
            .to_owned(),
        session_id: payload.get("session_id").cloned().unwrap_or(Value::Null),
        agent_id: json!(agent_id.unwrap_or("main")),
        agent_type,
        tool_use_id: payload.get("tool_use_id").cloned().unwrap_or(Value::Null),
        tool_name: payload.get("tool_name").cloned().unwrap_or(Value::Null),
        status: None,
        duration_ms: None,
        input: field("tool_input"),
        response: field("tool_response"),
        error: field("error"),
        prompt: field("prompt"),
        extra,
    }
}

/// A non-empty string field of `payload`.
fn text<'a>(payload: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// `value` unchanged, or `{truncated, original_length, head}` when its text exceeds [`MAX_FIELD`].
fn clip(value: &Value) -> Value {
    let text = match value {
        Value::Null => return Value::Null,
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let length = text.chars().count();
    if length <= MAX_FIELD {
        return value.clone();
    }
    let head: String = text.chars().take(MAX_FIELD).collect();
    json!({"truncated": true, "original_length": length, "head": head})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logged(dir: &Path, session: &str) -> Vec<Value> {
        let session: SessionId = session.parse().unwrap();
        Store::open(&session.db_path(dir))
            .unwrap()
            .events_after(0)
            .unwrap()
            .iter()
            .map(|row| serde_json::from_str(&row.json).unwrap())
            .collect()
    }

    #[test]
    fn clip_keeps_short_values_and_truncates_long_ones() {
        assert_eq!(clip(&json!("short")), json!("short"));
        assert_eq!(clip(&json!({"a": 1})), json!({"a": 1}));
        let long = "é".repeat(MAX_FIELD + 5);
        let clipped = clip(&json!(long));
        assert_eq!(clipped["truncated"], json!(true));
        assert_eq!(clipped["original_length"], json!(MAX_FIELD + 5));
        assert_eq!(clipped["head"].as_str().unwrap().chars().count(), MAX_FIELD);
    }

    #[test]
    fn pre_and_post_tool_use_are_paired_with_a_duration() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path();
        let pre = r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_use_id":"t1","tool_name":"Bash","tool_input":{"command":"ls"}}"#;
        let post = r#"{"hook_event_name":"PostToolUse","session_id":"s1","tool_use_id":"t1","tool_name":"Bash","tool_response":"ok"}"#;
        run(db, pre.as_bytes()).unwrap();
        run(db, post.as_bytes()).unwrap();

        let lines = logged(db, "s1");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["status"], json!("started"));
        assert_eq!(lines[0]["agent_id"], json!("main"));
        assert_eq!(lines[0]["agent_type"], json!("main"));
        assert_eq!(lines[1]["status"], json!("ok"));
        assert!(lines[1]["duration_ms"].as_i64().unwrap() >= 0);
    }

    #[test]
    fn post_without_a_logged_pre_has_no_duration() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path();
        let post = r#"{"hook_event_name":"PostToolUseFailure","session_id":"s1","tool_use_id":"t9","error":"boom"}"#;
        run(db, post.as_bytes()).unwrap();

        let lines = logged(db, "s1");
        assert_eq!(lines[0]["status"], json!("error"));
        assert_eq!(lines[0]["duration_ms"], Value::Null);
    }

    #[test]
    fn lifecycle_events_keep_transcript_path_and_subagents_have_no_default_type() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path();
        let start = r#"{"hook_event_name":"SubagentStart","session_id":"s2","agent_id":"a1","transcript_path":"/t.jsonl","cwd":"/x"}"#;
        run(db, start.as_bytes()).unwrap();

        let lines = logged(db, "s2");
        assert_eq!(lines[0]["agent_id"], json!("a1"));
        assert_eq!(lines[0]["agent_type"], Value::Null);
        assert_eq!(lines[0]["extra"]["transcript_path"], json!("/t.jsonl"));
        assert_eq!(lines[0]["extra"]["cwd"], json!("/x"));
    }

    #[test]
    fn each_session_gets_its_own_database_file() {
        let dir = tempfile::tempdir().unwrap();
        run(
            dir.path(),
            r#"{"hook_event_name":"Stop","session_id":"a"}"#.as_bytes(),
        )
        .unwrap();
        run(
            dir.path(),
            r#"{"hook_event_name":"Stop","session_id":"b"}"#.as_bytes(),
        )
        .unwrap();
        assert!(dir.path().join("a.db").exists() && dir.path().join("b.db").exists());
        assert_eq!(logged(dir.path(), "a").len(), 1);
    }

    #[test]
    fn a_session_id_that_is_not_a_file_name_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let bad = r#"{"hook_event_name":"Stop","session_id":"../escape"}"#;
        assert!(matches!(
            run(dir.path(), bad.as_bytes()),
            Err(HookError::Store(StoreError::InvalidSessionId(_)))
        ));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn empty_payload_goes_to_unknown_session() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path();
        run(db, "".as_bytes()).unwrap();
        assert_eq!(logged(db, "unknown-session")[0]["event"], json!("Unknown"));
    }
}
