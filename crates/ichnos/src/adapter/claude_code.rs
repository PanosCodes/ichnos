//! The Claude Code adapter: turn one Claude Code hook payload into Ichnos events.
//!
//! Wired for the events in [`HOOK_EVENTS`], which `ichnos install` registers. Claude Code's hook
//! event names are the Ichnos event names, so the payload maps field by field. Subagent tool calls carry
//! `agent_id`/`agent_type`, so the main agent and each subagent form separate lanes.
//!
//! Every payload also names the session's transcript. Each run logs the assistant messages added
//! to it since the last run, as `AssistantMessage` events; how far it has read is kept in the
//! session database's adapter state, so concurrent hook runs never log a line twice.

pub mod install;
mod tail;
mod tool_fields;
mod transcript;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use ichnos_core::log::{LogError, Logged, Logger, UNKNOWN_SESSION, text};
use ichnos_core::store::{SessionId, StoreError};
use serde_json::{Map, Value};
use thiserror::Error;

use self::transcript::Offsets;

/// The Claude Code hook events this adapter translates; `ichnos install` registers exactly these.
pub const HOOK_EVENTS: [&str; 9] = [
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "SubagentStart",
    "SubagentStop",
    "UserPromptSubmit",
    "SessionStart",
    "Stop",
    "Notification",
];

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

const NOTIFICATION_KEYS: [&str; 3] = ["message", "title", "notification_type"];

/// Payload key -> Ichnos event field, for the fields that are copied as they are.
const FIELDS: [(&str, &str); 9] = [
    ("hook_event_name", "event"),
    ("session_id", "session_id"),
    ("agent_id", "agent_id"),
    ("agent_type", "agent_type"),
    ("tool_use_id", "tool_use_id"),
    ("tool_name", "tool_name"),
    ("tool_input", "input"),
    ("tool_response", "response"),
    ("error", "error"),
];

/// Adapter state key holding the byte offsets read in each transcript file.
const OFFSETS_KEY: &str = "claude-code.transcript-offsets";

/// Why a payload or an import could not be logged.
#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("could not log the event")]
    Log(#[from] LogError),

    #[error("could not read or update the transcript position")]
    Store(#[from] StoreError),

    #[error("no transcript found for session {0}")]
    NoTranscript(SessionId),
}

/// Log one hook payload, received at `now`: the assistant messages written since the last run
/// first, then the hook event itself.
///
/// # Errors
///
/// Returns an [`AdapterError`] when the session id can't be a file name or the database can't be
/// written.
pub fn hook(
    logger: &mut Logger,
    payload: &Map<String, Value>,
    now: DateTime<Utc>,
) -> Result<Logged, AdapterError> {
    let session: Result<SessionId, _> = text(payload, "session_id")
        .unwrap_or(UNKNOWN_SESSION)
        .parse();
    if let (Some(path), Ok(session)) = (text(payload, "transcript_path"), &session) {
        for message in read_messages(logger, session, Path::new(path))? {
            logger.log(&message, now)?;
        }
    }
    Ok(logger.log(&translate(payload), now)?)
}

/// Log the assistant messages of a recorded session that aren't logged yet, from the transcript
/// its events name or, failing that, the one in Claude Code's projects folder. Returns how many
/// were logged.
///
/// # Errors
///
/// Returns [`AdapterError::NoTranscript`] when there is no transcript, or another
/// [`AdapterError`] when the database can't be read or written.
pub fn import(logger: &mut Logger, session: &SessionId) -> Result<usize, AdapterError> {
    // Don't create a database for a session that was never recorded.
    let recorded = if session.db_path(logger.sessions_dir()).exists() {
        recorded_transcript(logger, session)?
    } else {
        None
    };
    let path = recorded
        .or_else(|| transcript::find_in_projects(session))
        .ok_or_else(|| AdapterError::NoTranscript(session.clone()))?;
    let messages = read_messages(logger, session, &path)?;
    let now = Utc::now();
    for message in &messages {
        logger.log(message, now)?;
    }
    Ok(messages.len())
}

/// Add tool fields to the stored tool events and notifications of `session` that lack them, for
/// sessions recorded before the adapter wrote them. Returns how many records changed.
///
/// # Errors
///
/// Returns [`StoreError`] when the database can't be read or written; then nothing changes.
pub fn add_missing_tool_fields(
    logger: &mut Logger,
    session: &SessionId,
) -> Result<usize, StoreError> {
    let store = logger.store(session)?;
    let mut changed = Vec::new();
    for row in store.events_after(0)? {
        let Ok(Value::Object(mut record)) = serde_json::from_str(&row.json) else {
            continue;
        };
        let before = record.clone();
        add_tool_fields(&mut record);
        if record != before {
            let json = serde_json::to_string(&record).unwrap_or(row.json);
            changed.push((row.id, json));
        }
    }
    store.set_records(&changed)?;
    Ok(changed.len())
}

/// The Ichnos event for one hook payload, before `ichnos_core::log` fills in time and status.
fn translate(payload: &Map<String, Value>) -> Map<String, Value> {
    let mut event: Map<String, Value> = FIELDS
        .iter()
        .filter_map(|&(from, to)| Some((to.to_owned(), payload.get(from)?.clone())))
        .collect();
    if let Some(prompt) = payload.get("prompt") {
        event.insert("prompt".to_owned(), prompt.clone());
    }
    let notification: Map<String, Value> = NOTIFICATION_KEYS
        .iter()
        .filter_map(|&key| Some((key.to_owned(), payload.get(key)?.clone())))
        .collect();
    if !event.contains_key("input") && !notification.is_empty() {
        event.insert("input".to_owned(), Value::Object(notification));
    }
    let mut extra: Map<String, Value> = EXTRA_KEYS
        .iter()
        .filter_map(|&key| {
            payload
                .get(key)
                .map(|value| (key.to_owned(), value.clone()))
        })
        .collect();
    // Recorded on the lifecycle events only, to keep the per-tool lines small.
    if let (Some(path), None) = (payload.get("transcript_path"), text(payload, "tool_use_id")) {
        extra.insert("transcript_path".to_owned(), path.clone());
    }
    event.insert("extra".to_owned(), Value::Object(extra));
    add_tool_fields(&mut event);
    event
}

/// Add Ichnos's tool fields to a translated (or stored) event, from its own name, tool, input and
/// response; fields already there are replaced.
fn add_tool_fields(event: &mut Map<String, Value>) {
    let field = |name: &str| event.get(name).unwrap_or(&Value::Null);
    let fields = tool_fields::for_event(
        field("event").as_str().unwrap_or_default(),
        field("tool_name").as_str(),
        field("input"),
        field("response"),
    );
    event.extend(fields);
}

/// The assistant messages added to the transcript at `main` (and its subagents) since the last
/// read for `session`, claiming them in the same transaction that moves the offsets.
fn read_messages(
    logger: &mut Logger,
    session: &SessionId,
    main: &Path,
) -> Result<Vec<Map<String, Value>>, StoreError> {
    logger
        .store(session)?
        .update_adapter_state(OFFSETS_KEY, |old| {
            let mut offsets: Offsets = old
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
            let messages = transcript::new_messages(main, session.as_str(), &mut offsets);
            let saved = serde_json::to_string(&offsets).unwrap_or_else(|_| "{}".to_owned());
            (saved, messages)
        })
}

/// The last `transcript_path` the hook recorded for `session`.
fn recorded_transcript(
    logger: &mut Logger,
    session: &SessionId,
) -> Result<Option<PathBuf>, StoreError> {
    let rows = logger.store(session)?.events_after(0)?;
    Ok(rows.iter().rev().find_map(|row| {
        let record: Value = serde_json::from_str(&row.json).ok()?;
        let path = record.pointer("/extra/transcript_path")?.as_str()?;
        Some(PathBuf::from(path))
    }))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ichnos_core::store::Store;
    use serde_json::json;

    use super::*;

    fn payload(json: &str) -> Map<String, Value> {
        serde_json::from_str(json).unwrap()
    }

    fn run(dir: &Path, json: &str) -> Logged {
        hook(&mut Logger::new(dir), &payload(json), Utc::now()).unwrap()
    }

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
    fn a_tool_call_is_stored_as_the_record_the_hook_always_wrote() {
        let dir = tempfile::tempdir().unwrap();
        run(
            dir.path(),
            r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_use_id":"t1","tool_name":"Bash","tool_input":{"command":"ls"},"transcript_path":"/none.jsonl","cwd":"/x"}"#,
        );
        run(
            dir.path(),
            r#"{"hook_event_name":"PostToolUse","session_id":"s1","tool_use_id":"t1","tool_name":"Bash","tool_response":"ok"}"#,
        );
        let lines = logged(dir.path(), "s1");
        let keys: Vec<&str> = lines[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "ts",
                "epoch_ms",
                "event",
                "session_id",
                "agent_id",
                "agent_type",
                "tool_use_id",
                "tool_name",
                "status",
                "duration_ms",
                "input",
                "response",
                "error",
                "prompt",
                "extra",
                "label"
            ]
        );
        assert_eq!(lines[0]["input"], json!({"command": "ls"}));
        assert_eq!(lines[0]["label"], json!("ls"));
        assert_eq!(
            lines[0]["extra"],
            json!({"cwd": "/x"}),
            "tool events skip transcript_path"
        );
        assert_eq!(lines[1]["status"], json!("ok"));
        assert_eq!(lines[1]["response"], json!("ok"));
    }

    #[test]
    fn tool_fields_are_added_to_events_stored_without_them_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut logger = Logger::new(dir.path());
        for event in [
            json!({"event": "PreToolUse", "session_id": "s1", "tool_use_id": "t1", "tool_name": "Read",
                   "input": {"file_path": "/a.rs"}}),
            json!({"event": "PostToolUse", "session_id": "s1", "tool_use_id": "t2", "tool_name": "Agent",
                   "response": {"agentId": "a1"}}),
            json!({"event": "Stop", "session_id": "s1"}),
        ] {
            logger.log(event.as_object().unwrap(), Utc::now()).unwrap();
        }
        let session: SessionId = "s1".parse().unwrap();

        assert_eq!(add_missing_tool_fields(&mut logger, &session).unwrap(), 2);
        assert_eq!(add_missing_tool_fields(&mut logger, &session).unwrap(), 0);
        let lines = logged(dir.path(), "s1");
        assert_eq!(lines[0]["files"], json!([{"path": "/a.rs", "op": "read"}]));
        assert_eq!(
            lines[0]["input"],
            json!({"file_path": "/a.rs"}),
            "the rest is kept"
        );
        assert_eq!(lines[1]["spawned_agent"], json!("a1"));
        assert_eq!(lines[2].get("label"), None);
    }

    #[test]
    fn a_notification_keeps_its_message_as_input() {
        let dir = tempfile::tempdir().unwrap();
        run(
            dir.path(),
            r#"{"hook_event_name":"Notification","session_id":"s1","message":"Claude needs your permission to use Bash","notification_type":"permission_prompt","cwd":"/x"}"#,
        );
        let lines = logged(dir.path(), "s1");
        assert_eq!(
            lines[0]["input"],
            json!({"message": "Claude needs your permission to use Bash", "notification_type": "permission_prompt"})
        );
        assert_eq!(
            lines[0]["label"],
            json!("Claude needs your permission to use Bash")
        );
    }

    #[test]
    fn an_agent_call_records_the_subagent_it_started_and_an_edit_its_file() {
        let dir = tempfile::tempdir().unwrap();
        for payload in [
            r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_use_id":"t1","tool_name":"Edit","tool_input":{"file_path":"/a.rs","old_string":"x","new_string":"y"}}"#,
            r#"{"hook_event_name":"PostToolUse","session_id":"s1","tool_use_id":"t2","tool_name":"Agent","tool_response":{"agentId":"a1"}}"#,
        ] {
            run(dir.path(), payload);
        }
        let lines = logged(dir.path(), "s1");
        assert_eq!(lines[0]["files"], json!([{"path": "/a.rs", "op": "write"}]));
        assert_eq!(lines[1]["spawned_agent"], json!("a1"));
    }

    #[test]
    fn lifecycle_events_keep_transcript_path_and_subagents_have_no_default_type() {
        let dir = tempfile::tempdir().unwrap();
        run(
            dir.path(),
            r#"{"hook_event_name":"SubagentStart","session_id":"s2","agent_id":"a1","transcript_path":"/t.jsonl","cwd":"/x"}"#,
        );
        let lines = logged(dir.path(), "s2");
        assert_eq!(lines[0]["agent_id"], json!("a1"));
        assert_eq!(lines[0]["agent_type"], Value::Null);
        assert_eq!(lines[0]["extra"]["transcript_path"], json!("/t.jsonl"));
    }

    #[test]
    fn a_session_start_asks_for_a_viewer_unless_it_is_a_compaction() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            run(
                dir.path(),
                r#"{"hook_event_name":"SessionStart","session_id":"s1","source":"startup"}"#
            ),
            Logged::SessionStarted("s1".parse().unwrap())
        );
        assert_eq!(
            run(
                dir.path(),
                r#"{"hook_event_name":"SessionStart","session_id":"s1","source":"compact"}"#
            ),
            Logged::Stored
        );
    }

    #[test]
    fn new_transcript_messages_are_logged_once_before_the_hook_event() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("s1.jsonl");
        let line = |id: &str, text: &str| {
            format!(
                r#"{{"type":"assistant","timestamp":"2026-10-01T10:00:00Z","message":{{"id":"{id}","content":[{{"type":"text","text":"{text}"}}]}}}}"#
            ) + "\n"
        };
        fs::write(&transcript, line("m1", "first")).unwrap();
        let payload = format!(
            r#"{{"hook_event_name":"Stop","session_id":"s1","transcript_path":{transcript:?}}}"#
        );
        run(dir.path(), &payload);
        fs::write(&transcript, line("m1", "first") + &line("m2", "second")).unwrap();
        run(dir.path(), &payload);

        let events: Vec<(String, Value)> = logged(dir.path(), "s1")
            .into_iter()
            .map(|event| {
                (
                    event["event"].as_str().unwrap().to_owned(),
                    event["texts"].clone(),
                )
            })
            .collect();
        assert_eq!(
            events,
            [
                ("AssistantMessage".to_owned(), json!(["first"])),
                ("Stop".to_owned(), Value::Null),
                ("AssistantMessage".to_owned(), json!(["second"])),
                ("Stop".to_owned(), Value::Null),
            ]
        );
    }

    #[test]
    fn import_logs_the_messages_of_a_recorded_session_once() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("s1.jsonl");
        fs::write(
            &transcript,
            r#"{"type":"assistant","timestamp":"2026-10-01T10:00:00Z","message":{"id":"m1","content":[{"type":"text","text":"hi"}]}}"#.to_owned() + "\n",
        )
        .unwrap();
        let mut logger = Logger::new(dir.path());
        let session: SessionId = "s1".parse().unwrap();
        let start = json!({"event": "SessionStart", "session_id": "s1",
                           "extra": {"transcript_path": transcript}});
        logger.log(start.as_object().unwrap(), Utc::now()).unwrap();

        assert_eq!(import(&mut logger, &session).unwrap(), 1);
        assert_eq!(
            import(&mut logger, &session).unwrap(),
            0,
            "already imported"
        );
        assert!(matches!(
            import(&mut logger, &"nothing".parse().unwrap()),
            Err(AdapterError::NoTranscript(_))
        ));
    }

    #[test]
    fn an_empty_payload_goes_to_the_unknown_session() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), "{}");
        assert_eq!(
            logged(dir.path(), UNKNOWN_SESSION)[0]["event"],
            json!("Unknown")
        );
    }
}
