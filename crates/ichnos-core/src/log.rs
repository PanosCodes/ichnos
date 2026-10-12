//! The one way into Ichnos: store events in the Ichnos event format.
//!
//! An Ichnos event is a JSON object with an `event` name (`SessionStart`, `UserPromptSubmit`,
//! `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `SubagentStart`, `SubagentStop`, `Stop`,
//! `Notification`, `AssistantMessage` or `Debug`), a `session_id`, and the fields `Logger::log`
//! reads. A tool event may also carry the tool fields that [`crate::event`] describes (`label`,
//! `files`, `skill`, `todos`, `spawned_agent`); they are stored as they are, never clipped.
//! Where it came from doesn't matter here: adapters translate their tool's payloads into this
//! format, and anything else can write it directly.
//!
//! [`Logger::log`] fills in what the producer may leave out (time, the main agent), clips long
//! fields, pairs a tool call's `PostToolUse` with its `PreToolUse` for `status` and `duration_ms`,
//! and appends the record to the session's database. Each session gets its own database,
//! `<sessions_dir>/<session-id>.db`.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::debug;
use crate::event::MAIN_AGENT;
use crate::store::{NewEvent, SessionId, Store, StoreError};

/// Fields longer than this many characters are stored as `{truncated, original_length, head}`.
pub const MAX_FIELD: usize = 8000;

/// Session id used when an event names none.
pub const UNKNOWN_SESSION: &str = "unknown-session";

/// The fields [`Record`] names itself; every other field is kept as it is.
const NAMED_FIELDS: [&str; 15] = [
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
];

/// Why an event could not be logged.
#[derive(Debug, Error)]
pub enum LogError {
    #[error("could not store the event")]
    Store(#[from] StoreError),

    #[error("could not serialize the log record")]
    Serialize(#[source] serde_json::Error),
}

/// One stored event; field order is the order written.
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
    /// Fields of other event kinds, such as an `AssistantMessage`'s `texts`.
    #[serde(flatten)]
    rest: Map<String, Value>,
}

/// What the caller may do once an event is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Logged {
    /// The event started this session (from startup, resume or `/clear`); a caller may open a
    /// viewer on it.
    SessionStarted(SessionId),
    /// Nothing more to do; compactions also land here, since the session carries on.
    Stored,
}

/// Writes Ichnos events, keeping one open database per session it has seen.
#[derive(Debug)]
pub struct Logger {
    sessions_dir: PathBuf,
    stores: HashMap<SessionId, Store>,
}

impl Logger {
    /// A logger for the session databases in `sessions_dir`.
    #[must_use]
    pub fn new(sessions_dir: &Path) -> Self {
        Self {
            sessions_dir: sessions_dir.to_owned(),
            stores: HashMap::new(),
        }
    }

    /// The folder holding one database per session.
    #[must_use]
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    /// The database of `session`, opened (and created) on first use.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Open`] when the database can't be opened.
    pub fn store(&mut self, session: &SessionId) -> Result<&mut Store, StoreError> {
        Ok(match self.stores.entry(session.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                entry.insert(Store::open(&session.db_path(&self.sessions_dir))?)
            }
        })
    }

    /// Store one Ichnos event, received at `now`, in its session's database.
    ///
    /// # Errors
    ///
    /// Returns a [`LogError`] when the session id can't be a file name or the database can't be
    /// written.
    pub fn log(
        &mut self,
        event: &Map<String, Value>,
        now: DateTime<Utc>,
    ) -> Result<Logged, LogError> {
        let session: SessionId = text(event, "session_id")
            .unwrap_or(UNKNOWN_SESSION)
            .parse()?;
        let mut record = build_record(event, now);
        let tool_use_id = text(event, "tool_use_id");
        let store = self.store(&session)?;

        match record.event.as_str() {
            "PreToolUse" => record.status = Some("started"),
            "PostToolUse" | "PostToolUseFailure" => {
                record.status = Some(if record.event == "PostToolUseFailure" {
                    "error"
                } else {
                    "ok"
                });
                if let Some(id) = tool_use_id {
                    record.duration_ms =
                        store.start_ms(id)?.map(|started| record.epoch_ms - started);
                }
            }
            _ => {}
        }

        let json = serde_json::to_string(&record).map_err(LogError::Serialize)?;
        store.add_event(&NewEvent {
            epoch_ms: record.epoch_ms,
            event: &record.event,
            tool_use_id,
            json: &json,
        })?;
        let compaction = record.extra.get("source").and_then(Value::as_str) == Some("compact");
        if record.event == "SessionStart" && !compaction {
            Ok(Logged::SessionStarted(session))
        } else {
            Ok(Logged::Stored)
        }
    }
}

/// The record for `event` received at `now`, before status and duration are filled in.
fn build_record(event: &Map<String, Value>, now: DateTime<Utc>) -> Record {
    let epoch_ms = event
        .get("epoch_ms")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| now.timestamp_millis());
    let ts = match text(event, "ts") {
        Some(ts) => ts.to_owned(),
        None => DateTime::from_timestamp_millis(epoch_ms)
            .unwrap_or(now)
            .to_rfc3339_opts(SecondsFormat::Millis, false),
    };
    let agent_id = text(event, "agent_id");
    let agent_type = match text(event, "agent_type") {
        Some(kind) => json!(kind),
        None if agent_id.is_none() => json!(MAIN_AGENT),
        None => Value::Null,
    };
    let mut extra = match event.get("extra") {
        Some(Value::Object(extra)) => extra
            .iter()
            .map(|(key, value)| (key.clone(), clip(value)))
            .collect(),
        _ => Map::new(),
    };
    let rest = event
        .iter()
        .filter(|(key, _)| !NAMED_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    expand_debug(&mut extra);
    let raw = |key: &str| event.get(key).cloned().unwrap_or(Value::Null);
    let field = |key: &str| event.get(key).map_or(Value::Null, clip);

    Record {
        ts,
        epoch_ms,
        event: text(event, "event").unwrap_or("Unknown").to_owned(),
        session_id: raw("session_id"),
        agent_id: json!(agent_id.unwrap_or(MAIN_AGENT)),
        agent_type,
        tool_use_id: raw("tool_use_id"),
        tool_name: raw("tool_name"),
        status: None,
        duration_ms: None,
        input: field("input"),
        response: field("response"),
        error: field("error"),
        prompt: field("prompt"),
        extra,
        rest,
    }
}

/// Expands an account in the compact form in `extra.debug` (a `Debug` event) to the full form,
/// or marks it invalid with what broke it.
fn expand_debug(extra: &mut Map<String, Value>) {
    if let Some(account) = extra.get_mut("debug")
        && account.get("v").and_then(Value::as_u64) == Some(debug::compact::VERSION)
    {
        *account = match debug::compact::expand(account) {
            Ok(full) => full,
            Err(error) => json!({
                "invalid": true,
                "raw": account.to_string().chars().take(500).collect::<String>(),
                "error": error,
            }),
        };
    }
}

/// A non-empty string field of `event`.
#[must_use]
pub fn text<'a>(event: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    event
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

    fn object(value: &Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
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

    fn log_all(dir: &Path, events: &[Value]) -> Vec<Logged> {
        let mut logger = Logger::new(dir);
        events
            .iter()
            .map(|event| logger.log(&object(event), Utc::now()).unwrap())
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
        log_all(
            dir.path(),
            &[
                json!({"event": "PreToolUse", "session_id": "s1", "tool_use_id": "t1", "tool_name": "Bash", "input": {"command": "ls"}}),
                json!({"event": "PostToolUse", "session_id": "s1", "tool_use_id": "t1", "response": "ok"}),
            ],
        );
        let lines = logged(dir.path(), "s1");
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
        log_all(
            dir.path(),
            &[
                json!({"event": "PostToolUseFailure", "session_id": "s1", "tool_use_id": "t9", "error": "boom"}),
            ],
        );
        let lines = logged(dir.path(), "s1");
        assert_eq!(lines[0]["status"], json!("error"));
        assert_eq!(lines[0]["duration_ms"], Value::Null);
    }

    #[test]
    fn subagents_have_no_default_type_and_extra_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        log_all(
            dir.path(),
            &[
                json!({"event": "SubagentStart", "session_id": "s2", "agent_id": "a1", "extra": {"cwd": "/x"}}),
            ],
        );
        let lines = logged(dir.path(), "s2");
        assert_eq!(lines[0]["agent_id"], json!("a1"));
        assert_eq!(lines[0]["agent_type"], Value::Null);
        assert_eq!(lines[0]["extra"]["cwd"], json!("/x"));
    }

    #[test]
    fn a_given_time_is_kept_and_other_fields_pass_through() {
        let dir = tempfile::tempdir().unwrap();
        log_all(
            dir.path(),
            &[
                json!({"event": "AssistantMessage", "session_id": "s1", "epoch_ms": 1_000, "message_id": "m1", "texts": ["hi"]}),
            ],
        );
        let lines = logged(dir.path(), "s1");
        assert_eq!(lines[0]["epoch_ms"], json!(1_000));
        assert_eq!(lines[0]["ts"], json!("1970-01-01T00:00:01.000+00:00"));
        assert_eq!(lines[0]["texts"], json!(["hi"]));
        assert_eq!(lines[0]["message_id"], json!("m1"));
    }

    #[test]
    fn a_compact_account_in_a_debug_event_is_stored_expanded() {
        let dir = tempfile::tempdir().unwrap();
        let account = json!({"v": 2, "o": {"s": "done", "r": ["docs"], "u": [], "k": []}});
        log_all(
            dir.path(),
            &[
                json!({"event": "Debug", "session_id": "s1", "extra": {"turn_id": "t1", "debug": account}}),
                json!({"event": "Debug", "session_id": "s1", "extra": {"debug": {"v": 2, "o": {"s": "maybe"}}}}),
            ],
        );
        let lines = logged(dir.path(), "s1");
        assert_eq!(
            lines[0]["extra"]["debug"],
            json!({"v": 1, "outcome": {"state": "done", "remaining": ["docs"], "unverified_claims": [], "risks": []}})
        );
        assert_eq!(lines[0]["extra"]["turn_id"], json!("t1"));
        assert_eq!(lines[1]["extra"]["debug"]["invalid"], json!(true));
        assert!(
            lines[1]["extra"]["debug"]["error"]
                .as_str()
                .unwrap()
                .contains("outcome.state")
        );
    }

    #[test]
    fn each_session_gets_its_own_database_file() {
        let dir = tempfile::tempdir().unwrap();
        log_all(
            dir.path(),
            &[
                json!({"event": "Stop", "session_id": "a"}),
                json!({"event": "Stop", "session_id": "b"}),
            ],
        );
        assert!(dir.path().join("a.db").exists() && dir.path().join("b.db").exists());
        assert_eq!(logged(dir.path(), "a").len(), 1);
    }

    #[test]
    fn a_session_id_that_is_not_a_file_name_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let bad = object(&json!({"event": "Stop", "session_id": "../escape"}));
        assert!(matches!(
            Logger::new(dir.path()).log(&bad, Utc::now()),
            Err(LogError::Store(StoreError::InvalidSessionId(_)))
        ));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_session_start_asks_for_a_viewer_unless_it_is_a_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let results = log_all(
            dir.path(),
            &[
                json!({"event": "SessionStart", "session_id": "s1", "extra": {"source": "startup"}}),
                json!({"event": "SessionStart", "session_id": "s1", "extra": {"source": "compact"}}),
                json!({"event": "Stop", "session_id": "s1"}),
            ],
        );
        assert_eq!(
            results,
            [
                Logged::SessionStarted("s1".parse().unwrap()),
                Logged::Stored,
                Logged::Stored
            ]
        );
    }

    #[test]
    fn an_empty_event_goes_to_the_unknown_session() {
        let dir = tempfile::tempdir().unwrap();
        log_all(dir.path(), &[json!({})]);
        assert_eq!(
            logged(dir.path(), UNKNOWN_SESSION)[0]["event"],
            json!("Unknown")
        );
    }
}
