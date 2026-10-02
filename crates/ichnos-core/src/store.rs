//! The SQLite databases that hold logged events and summaries: one file per session,
//! `~/.ichnos/sessions/<session-id>.db`.
//!
//! The only module that knows SQL; the schema itself is in `schema.sql` at this crate's root. Every
//! event row keeps the full record exactly as the viewer receives it (`record`), next to a few
//! indexed columns for lookups. One [`Store`] wraps one
//! connection and is not `Sync`, so every process or task opens its own; WAL mode and a busy
//! timeout let the hook processes and the viewer write and read concurrently.
//!
//! A summary points at the event row that starts its entry (`event_id`); the viewer maps row ids
//! to its own entries, so the database never needs to know how the viewer groups events.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// How long a write waits for another writer's lock before failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Version that `schema.sql` sets in `PRAGMA user_version`. Version 2 keys summaries by event
/// row id; version 3 adds `adapter_state`.
const SCHEMA_VERSION: i64 = 3;

/// Statements run on a version 1 database. Version 1 keyed summaries by a viewer-specific string,
/// so that table is dropped; summaries are a cache that the summarizer rebuilds.
const MIGRATE_FROM_V1: &str = "DROP TABLE IF EXISTS summaries;";

/// The current schema; also sets `user_version` to [`SCHEMA_VERSION`].
const SCHEMA: &str = include_str!("../schema.sql");

/// Why a database operation failed.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("could not open the database at {path}")]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error("database query failed")]
    Query(#[from] rusqlite::Error),

    #[error("invalid session id {0:?}: use ASCII letters, digits, '_' and '-' only")]
    InvalidSessionId(String),
}

/// Whether `text` is non-empty and only ASCII letters, digits, `_` and `-`: safe in file names.
fn is_safe_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// A session id made of ASCII letters, digits, `_` and `-`; it names the session's database file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct SessionId(String);

impl SessionId {
    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// This session's database inside the sessions folder `dir`: `<dir>/<id>.db`.
    #[must_use]
    pub fn db_path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.db", self.0))
    }
}

impl FromStr for SessionId {
    type Err = StoreError;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        if is_safe_name(id) {
            Ok(Self(id.to_owned()))
        } else {
            Err(StoreError::InvalidSessionId(id.to_owned()))
        }
    }
}

impl TryFrom<String> for SessionId {
    type Error = StoreError;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        id.parse()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The longest description [`Overview`] keeps, in characters.
pub const DESCRIPTION_CHARS: usize = 160;

/// What the sessions list shows about one session, read straight from its database.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Overview {
    /// When the first event arrived.
    pub start_ms: Option<i64>,
    /// When the last event arrived.
    pub end_ms: Option<i64>,
    /// Every logged event except model messages.
    pub events: i64,
    /// Tool calls started.
    pub calls: i64,
    /// Tool calls that failed.
    pub errors: i64,
    /// The working directory the agent ran in.
    pub cwd: Option<String>,
    /// The session's first prompt on one line, cut to [`DESCRIPTION_CHARS`]; none before a prompt.
    pub description: Option<String>,
}

impl Overview {
    /// Read the overview of the session database at `path`, without writing to it or creating it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Open`] when the file can't be opened, or [`StoreError::Query`] when it
    /// isn't a ichnos database.
    pub fn read(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(
            |source| StoreError::Open {
                path: path.to_owned(),
                source,
            },
        )?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        let mut overview = conn.query_row(
            "SELECT MIN(epoch_ms), MAX(epoch_ms), COALESCE(SUM(event <> 'AssistantMessage'), 0),
                    COALESCE(SUM(event = 'PreToolUse'), 0), COALESCE(SUM(event = 'PostToolUseFailure'), 0)
             FROM events",
            [],
            |row| {
                Ok(Self {
                    start_ms: row.get(0)?,
                    end_ms: row.get(1)?,
                    events: row.get(2)?,
                    calls: row.get(3)?,
                    errors: row.get(4)?,
                    ..Self::default()
                })
            },
        )?;
        let first_text = |sql: &str| -> rusqlite::Result<Option<String>> {
            Ok(conn
                .query_row(sql, [], |row| row.get::<_, Option<String>>(0))
                .optional()?
                .flatten())
        };
        overview.cwd = first_text(
            "SELECT json_extract(record, '$.extra.cwd') AS cwd FROM events
             WHERE cwd IS NOT NULL ORDER BY id LIMIT 1",
        )?;
        // A prompt over the field limit is stored as `{truncated, original_length, head}`.
        let prompt = first_text(
            "SELECT COALESCE(json_extract(record, '$.prompt.head'), json_extract(record, '$.prompt')) AS prompt
             FROM events WHERE event = 'UserPromptSubmit' AND prompt <> '' ORDER BY id LIMIT 1",
        )?;
        overview.description = prompt.and_then(|text| one_line(&text, DESCRIPTION_CHARS));
        Ok(overview)
    }
}

/// `text` with runs of whitespace collapsed to one space, cut to `max` characters with a trailing
/// `…`; none when nothing is left.
fn one_line(text: &str, max: usize) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= max {
        return Some(line);
    }
    let mut cut: String = line.chars().take(max).collect();
    cut.push('…');
    Some(cut)
}

/// One stored event or summary, already serialized as the JSON the viewer receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Row id; increases with every insert, so it doubles as a resume cursor.
    pub id: i64,
    /// The event record, or `{"event_id", "summary"}` for a summary.
    pub json: String,
}

/// An event to append; `json` is the full record exactly as the viewer receives it.
#[derive(Debug, Clone, Copy)]
pub struct NewEvent<'a> {
    /// When the event happened.
    pub epoch_ms: i64,
    /// The Ichnos event name, such as `PreToolUse`.
    pub event: &'a str,
    /// Pairs a tool call's Pre and Post events.
    pub tool_use_id: Option<&'a str>,
    /// The serialized record.
    pub json: &'a str,
}

/// One connection to the database.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the database at `path`, in WAL mode with the schema in place.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Open`] when the file can't be opened or the schema can't be created.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let open_error = |source| StoreError::Open {
            path: path.to_owned(),
            source,
        };
        let mut conn = Connection::open(path).map_err(open_error)?;
        conn.busy_timeout(BUSY_TIMEOUT).map_err(open_error)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(open_error)?;
        // With WAL, NORMAL can only lose the last transactions on power loss, never corrupt.
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(open_error)?;
        migrate(&mut conn).map_err(open_error)?;
        Ok(Self { conn })
    }

    /// Append one event; returns its row id.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] when the insert fails.
    pub fn add_event(&self, event: &NewEvent) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO events (epoch_ms, event, tool_use_id, record) VALUES (?1, ?2, ?3, ?4)",
            params![event.epoch_ms, event.event, event.tool_use_id, event.json],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// `epoch_ms` of the `PreToolUse` that started `tool_use_id`, if it was logged.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] when the lookup fails.
    pub fn start_ms(&self, tool_use_id: &str) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT epoch_ms FROM events WHERE tool_use_id = ?1 AND event = 'PreToolUse' ORDER BY id DESC LIMIT 1",
                [tool_use_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Events with a row id above `after_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] when the query fails.
    pub fn events_after(&self, after_id: i64) -> Result<Vec<Row>, StoreError> {
        self.rows(
            "SELECT id, record FROM events WHERE id > ?1 ORDER BY id",
            after_id,
        )
    }

    /// Summaries with a row id above `after_id`, as `{"event_id", "summary"}` JSON.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] when the query fails.
    pub fn summaries_after(&self, after_id: i64) -> Result<Vec<Row>, StoreError> {
        self.rows(
            "SELECT id, json_object('event_id', event_id, 'summary', summary) FROM summaries WHERE id > ?1 ORDER BY id",
            after_id,
        )
    }

    fn rows(&self, sql: &str, after_id: i64) -> Result<Vec<Row>, StoreError> {
        let mut query = self.conn.prepare_cached(sql)?;
        let rows = query.query_map([after_id], |row| {
            Ok(Row {
                id: row.get(0)?,
                json: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Update the adapter value under `key` in one immediate transaction: `update` gets the
    /// current value and returns the new one, plus anything the caller wants back. Two hook
    /// processes running at once see each other's updates in order, never the same old value.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] when the value can't be read or written; then it is unchanged.
    pub fn update_adapter_state<T>(
        &mut self,
        key: &str,
        update: impl FnOnce(Option<String>) -> (String, T),
    ) -> Result<T, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<String> = tx
            .query_row(
                "SELECT value FROM adapter_state WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?;
        let (new, result) = update(old);
        tx.execute(
            "INSERT INTO adapter_state (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, new],
        )?;
        tx.commit()?;
        Ok(result)
    }

    /// Event row ids that already have a summary.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] when the query fails.
    pub fn summarized_event_ids(&self) -> Result<HashSet<i64>, StoreError> {
        let mut query = self.conn.prepare_cached("SELECT event_id FROM summaries")?;
        let keys = query.query_map([], |row| row.get(0))?;
        Ok(keys.collect::<Result<_, _>>()?)
    }

    /// Store `(event_id, summary)` pairs; events that already have one are left alone.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] when an insert fails; then none are kept.
    pub fn add_summaries(&mut self, rows: &[(i64, String)]) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        {
            let mut insert =
                tx.prepare("INSERT OR IGNORE INTO summaries (event_id, summary) VALUES (?1, ?2)")?;
            for (event_id, summary) in rows {
                insert.execute(params![event_id, summary])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

/// Bring the schema up to [`SCHEMA_VERSION`]. The common case (already current) is one read; an
/// upgrade runs in an immediate transaction so concurrent openers don't migrate twice.
fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let version = |conn: &Connection| {
        conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
    };
    if version(conn)? == SCHEMA_VERSION {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let found = version(&tx)?;
    if found < 2 {
        tx.execute_batch(MIGRATE_FROM_V1)?;
    }
    if found < SCHEMA_VERSION {
        tx.execute_batch(SCHEMA)?;
    }
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("test.db")).unwrap();
        (dir, store)
    }

    fn session(id: &str) -> SessionId {
        id.parse().unwrap()
    }

    fn event<'a>(
        epoch_ms: i64,
        event: &'a str,
        tool_use_id: Option<&'a str>,
        json: &'a str,
    ) -> NewEvent<'a> {
        NewEvent {
            epoch_ms,
            event,
            tool_use_id,
            json,
        }
    }

    #[test]
    fn schema_file_sets_the_version_the_code_expects() {
        let (_dir, store) = store();
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn a_session_is_one_file_named_after_it() {
        assert_eq!(
            session("s1").db_path(Path::new("/d")),
            PathBuf::from("/d/s1.db")
        );
    }

    #[test]
    fn session_ids_are_validated() {
        assert_eq!(session("1a2e0de0-d800_x").as_str(), "1a2e0de0-d800_x");
        for bad in ["", "../etc/passwd", "a b", "é"] {
            assert!(matches!(
                bad.parse::<SessionId>(),
                Err(StoreError::InvalidSessionId(_))
            ));
        }
    }

    #[test]
    fn events_are_read_back_after_a_cursor() {
        let (_dir, store) = store();
        let first = store
            .add_event(&event(10, "UserPromptSubmit", None, "{\"n\":1}"))
            .unwrap();
        store
            .add_event(&event(30, "Stop", None, "{\"n\":2}"))
            .unwrap();

        let all: Vec<String> = store
            .events_after(0)
            .unwrap()
            .into_iter()
            .map(|row| row.json)
            .collect();
        assert_eq!(all, vec!["{\"n\":1}", "{\"n\":2}"]);
        let rest = store.events_after(first).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].json, "{\"n\":2}");
    }

    #[test]
    fn start_ms_finds_the_pre_tool_use() {
        let (_dir, store) = store();
        store
            .add_event(&event(100, "PreToolUse", Some("t1"), "{}"))
            .unwrap();
        store
            .add_event(&event(150, "PostToolUse", Some("t1"), "{}"))
            .unwrap();
        assert_eq!(store.start_ms("t1").unwrap(), Some(100));
        assert_eq!(store.start_ms("missing").unwrap(), None);
    }

    #[test]
    fn adapter_state_starts_empty_and_keeps_what_the_update_returns() {
        let (_dir, mut store) = store();
        let seen = store
            .update_adapter_state("a.offset", |old| ("10".to_owned(), old))
            .unwrap();
        assert_eq!(seen, None);
        let seen = store
            .update_adapter_state("a.offset", |old| ("20".to_owned(), old))
            .unwrap();
        assert_eq!(seen.as_deref(), Some("10"));
    }

    #[test]
    fn a_version_2_database_keeps_its_summaries_when_upgraded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (id INTEGER PRIMARY KEY, epoch_ms INTEGER NOT NULL, event TEXT NOT NULL,
                                  tool_use_id TEXT, record TEXT NOT NULL);
             CREATE TABLE summaries (id INTEGER PRIMARY KEY, event_id INTEGER NOT NULL UNIQUE,
                                     summary TEXT NOT NULL);
             INSERT INTO summaries (event_id, summary) VALUES (1, 'Kept.');
             PRAGMA user_version = 2;",
        )
        .unwrap();
        drop(conn);

        let mut store = Store::open(&path).unwrap();
        assert_eq!(store.summarized_event_ids().unwrap().len(), 1);
        store
            .update_adapter_state("k", |_| ("v".to_owned(), ()))
            .unwrap();
    }

    #[test]
    fn summaries_keep_the_first_one_per_event() {
        let (_dir, mut store) = store();
        let (e1, e2) = (
            store.add_event(&event(1, "A", None, "{}")).unwrap(),
            store.add_event(&event(2, "B", None, "{}")).unwrap(),
        );
        store.add_summaries(&[(e1, "First.".into())]).unwrap();
        store
            .add_summaries(&[(e1, "Again.".into()), (e2, "Two \"quoted\".".into())])
            .unwrap();

        assert_eq!(
            store.summarized_event_ids().unwrap(),
            HashSet::from([e1, e2])
        );
        let rows = store.summaries_after(0).unwrap();
        let parsed: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| serde_json::from_str(&row.json).unwrap())
            .collect();
        assert_eq!(
            parsed,
            vec![
                serde_json::json!({"event_id": e1, "summary": "First."}),
                serde_json::json!({"event_id": e2, "summary": "Two \"quoted\"."})
            ]
        );
        assert_eq!(store.summaries_after(rows[1].id).unwrap(), Vec::new());
    }

    #[test]
    fn overview_counts_the_session_and_describes_it_by_its_first_prompt() {
        let (dir, store) = store();
        let empty = Overview::read(&dir.path().join("test.db")).unwrap();
        assert_eq!(empty, Overview::default());

        let prompt = format!(r#"{{"prompt":"  fix\n  the   bug {}"}}"#, "x".repeat(300));
        let rows = [
            event(10, "SessionStart", None, r#"{"extra":{"cwd":"/work/app"}}"#),
            event(20, "UserPromptSubmit", None, &prompt),
            event(30, "UserPromptSubmit", None, r#"{"prompt":"second"}"#),
            event(40, "PreToolUse", Some("t1"), "{}"),
            event(45, "AssistantMessage", None, "{}"),
            event(50, "PostToolUseFailure", Some("t1"), "{}"),
            event(60, "PreToolUse", Some("t2"), "{}"),
        ];
        for row in &rows {
            store.add_event(row).unwrap();
        }
        let overview = Overview::read(&dir.path().join("test.db")).unwrap();
        assert_eq!(
            (overview.start_ms, overview.end_ms, overview.events),
            (Some(10), Some(60), 6)
        );
        assert_eq!((overview.calls, overview.errors), (2, 1));
        assert_eq!(overview.cwd.as_deref(), Some("/work/app"));
        let description = overview.description.unwrap();
        assert!(description.starts_with("fix the bug xxx"));
        assert_eq!(description.chars().count(), DESCRIPTION_CHARS + 1);
        assert!(description.ends_with('…'));
    }

    #[test]
    fn overview_reads_a_prompt_that_was_clipped_to_its_head() {
        let (dir, store) = store();
        let clipped = r#"{"prompt":{"truncated":true,"original_length":9999,"head":"the start"}}"#;
        store
            .add_event(&event(1, "UserPromptSubmit", None, clipped))
            .unwrap();
        let overview = Overview::read(&dir.path().join("test.db")).unwrap();
        assert_eq!(overview.description.as_deref(), Some("the start"));
    }

    #[test]
    fn overview_of_a_file_that_is_not_a_database_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("junk.db");
        std::fs::write(
            &path,
            "this is not sqlite, and it is long enough to be read as a header",
        )
        .unwrap();
        assert!(Overview::read(&path).is_err());
    }

    #[test]
    fn opening_a_version_1_database_replaces_its_summaries_and_keeps_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let v1 = Connection::open(&path).unwrap();
        v1.execute_batch(
            "CREATE TABLE events (id INTEGER PRIMARY KEY, epoch_ms INTEGER NOT NULL, event TEXT NOT NULL,
                                  tool_use_id TEXT, record TEXT NOT NULL);
             INSERT INTO events (epoch_ms, event, record) VALUES (1, 'Stop', '{}');
             CREATE TABLE summaries (id INTEGER PRIMARY KEY, key TEXT NOT NULL UNIQUE,
                                     summary TEXT NOT NULL);
             INSERT INTO summaries (key, summary) VALUES ('Stop:1:main', 'Old.');",
        )
        .unwrap();
        drop(v1);

        let mut store = Store::open(&path).unwrap();
        assert_eq!(store.events_after(0).unwrap().len(), 1);
        assert_eq!(store.summarized_event_ids().unwrap(), HashSet::new());
        store.add_summaries(&[(1, "New.".into())]).unwrap();
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.summarized_event_ids().unwrap(), HashSet::from([1]));
    }
}
