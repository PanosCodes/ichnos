//! The sessions folder as a list: one [`SessionEntry`] per `<session-id>.db`, newest change first.
//!
//! A [`Catalog`] remembers what it read from each database and reads a database again only when
//! its modification time changes, so listing every few seconds stays cheap.

use std::collections::HashMap;
use std::error::Error as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::store::{Overview, SessionId};

/// Each database's overview, or why it couldn't be read, with the modification time it was read at.
type Overviews = HashMap<PathBuf, (u128, Result<Overview, String>)>;

/// One session in the sessions folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionEntry {
    id: SessionId,
    /// When the session's database last changed.
    mtime_ms: u128,
    /// The database file, so a viewer can tell an agent where to query it.
    path: PathBuf,
    #[serde(flatten)]
    overview: Overview,
    /// Why the database couldn't be read; the overview is then empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl SessionEntry {
    /// The session's id.
    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// When the session's database last changed, in epoch milliseconds.
    #[must_use]
    pub fn mtime_ms(&self) -> u128 {
        self.mtime_ms
    }

    /// The session's database file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the database says about the session; empty when it couldn't be read.
    #[must_use]
    pub fn overview(&self) -> &Overview {
        &self.overview
    }

    /// Why the database couldn't be read, if it couldn't.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// Lists the sessions folder, rereading only the databases that changed since the last listing.
#[derive(Debug)]
pub struct Catalog {
    /// The sessions folder holding one `<session-id>.db` per session.
    dir: PathBuf,
    cache: Overviews,
}

impl Catalog {
    /// A catalog of the sessions folder `dir`; nothing is read until [`Catalog::list`].
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            cache: Overviews::new(),
        }
    }

    /// The sessions folder this catalog lists.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The sessions, newest change first.
    ///
    /// A session's newest writes may sit in its `-wal` file, so the later of the two modification
    /// times counts. Afterwards the cache holds just the sessions listed.
    ///
    /// # Errors
    ///
    /// Returns the [`io::Error`] when the sessions folder can't be read. A database that can't be
    /// read is listed with its [`SessionEntry::error`] instead.
    pub fn list(&mut self) -> io::Result<Vec<SessionEntry>> {
        let mut kept = Overviews::new();
        let mut sessions: Vec<SessionEntry> = fs::read_dir(&self.dir)?
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let id: SessionId = name.strip_suffix(".db")?.parse().ok()?;
                let path = entry.path();
                let wal = self.dir.join(format!("{id}.db-wal"));
                let mtime_ms = modified_ms(&path).max(modified_ms(&wal));
                let read = match self.cache.remove(&path) {
                    Some((at, read)) if at == mtime_ms => read,
                    _ => Overview::read(&path).map_err(|err| match err.source() {
                        Some(source) => format!("{err}: {source}"),
                        None => err.to_string(),
                    }),
                };
                kept.insert(path.clone(), (mtime_ms, read.clone()));
                let (overview, error) = match read {
                    Ok(overview) => (overview, None),
                    Err(error) => (Overview::default(), Some(error)),
                };
                Some(SessionEntry {
                    id,
                    mtime_ms,
                    path,
                    overview,
                    error,
                })
            })
            .collect();
        self.cache = kept;
        sessions.sort_by_key(|session| std::cmp::Reverse(session.mtime_ms));
        Ok(sessions)
    }
}

/// How long after its last change a session still counts as being written to.
pub const RECENT_MS: u128 = 30_000;

impl SessionEntry {
    /// Whether every word of `query` appears in the description, folder or id, ignoring case.
    #[must_use]
    pub fn matches(&self, query: &str) -> bool {
        let haystack = [
            self.overview.description.as_deref(),
            self.overview.cwd.as_deref(),
            Some(self.id.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
        query
            .split_whitespace()
            .all(|word| haystack.contains(&word.to_lowercase()))
    }

    /// The facts under a switcher row's description: folder, start, length and calls, as known.
    #[must_use]
    pub fn facts(&self) -> Vec<String> {
        let mut facts = Vec::new();
        let project = project_of(self.overview.cwd.as_deref().unwrap_or_default());
        if !project.is_empty() {
            facts.push(project);
        }
        let mtime = i64::try_from(self.mtime_ms).unwrap_or(i64::MAX);
        let when = self.overview.start_ms.unwrap_or(mtime);
        facts.extend(crate::local_time(when, "%b %-d, %H:%M"));
        if let (Some(start), Some(end)) = (self.overview.start_ms, self.overview.end_ms) {
            facts.push(span_label(end - start));
        }
        match self.overview.calls {
            0 => {}
            1 => facts.push("1 call".to_owned()),
            calls => facts.push(format!("{calls} calls")),
        }
        facts
    }

    /// Whether the session changed within [`RECENT_MS`] of `now_ms`, so it is likely still going.
    #[must_use]
    pub fn is_recent(&self, now_ms: u128) -> bool {
        now_ms.saturating_sub(self.mtime_ms) < RECENT_MS
    }
}

/// The sessions that match `query`, in their given order.
pub fn filter_sessions<'a>(
    sessions: &'a [SessionEntry],
    query: &'a str,
) -> impl Iterator<Item = &'a SessionEntry> {
    sessions
        .iter()
        .filter(move |session| session.matches(query))
}

/// A folder's last part, with its parent when the name alone says little (`trees/1`).
#[must_use]
pub fn project_of(cwd: &str) -> String {
    let parts: Vec<&str> = cwd.split('/').filter(|part| !part.is_empty()).collect();
    let keep = match parts.last() {
        Some(last) if last.chars().count() < 4 => 2,
        _ => 1,
    };
    parts[parts.len().saturating_sub(keep)..].join("/")
}

/// A span of milliseconds as `45s`, `12m` or `2h 05m`.
#[must_use]
pub fn span_label(ms: i64) -> String {
    let seconds = ((ms + 500) / 1000).max(0);
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

/// When `path` was last modified, in epoch milliseconds; 0 when that can't be read.
fn modified_ms(path: &Path) -> u128 {
    path.metadata()
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_millis())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::store::{NewEvent, Store};

    #[test]
    fn sessions_are_the_database_files_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["old.db", "new.db", "new.db-wal", "notes.txt", "bad name.db"] {
            fs::write(dir.path().join(name), "").unwrap();
        }
        let old = fs::File::options()
            .write(true)
            .open(dir.path().join("old.db"))
            .unwrap();
        old.set_modified(UNIX_EPOCH + Duration::from_secs(1))
            .unwrap();

        let sessions = Catalog::new(dir.path()).list().unwrap();
        let ids: Vec<&str> = sessions.iter().map(|s| s.id().as_str()).collect();
        assert_eq!(ids, vec!["new", "old"]);
        assert_eq!(sessions[0].path(), dir.path().join("new.db"));
    }

    #[test]
    fn a_session_that_cannot_be_read_is_listed_with_its_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("bad.db"),
            "not a database, just a long enough line of text",
        )
        .unwrap();

        let sessions = Catalog::new(dir.path()).list().unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].error().is_some());
        assert_eq!(sessions[0].overview(), &Overview::default());
    }

    #[test]
    fn a_database_is_read_again_only_when_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.db");
        let store = Store::open(&path).unwrap();
        let prompt = |text: &str| format!(r#"{{"prompt":"{text}"}}"#);
        let add = |text: &str| {
            store
                .add_event(&NewEvent {
                    epoch_ms: 1,
                    event: "UserPromptSubmit",
                    tool_use_id: None,
                    json: &prompt(text),
                })
                .unwrap();
        };
        add("first");
        let mut catalog = Catalog::new(dir.path());
        let listed = catalog.list().unwrap();
        assert_eq!(listed[0].overview().description.as_deref(), Some("first"));

        // Same modification time: the cached read is served, even though the data moved on.
        let (at, read) = catalog.cache.remove(&path).unwrap();
        let mut stale = read.unwrap();
        stale.description = Some("cached".into());
        catalog.cache.insert(path.clone(), (at, Ok(stale)));
        let listed = catalog.list().unwrap();
        assert_eq!(listed[0].overview().description.as_deref(), Some("cached"));

        // A different modification time: it is read again.
        catalog.cache.get_mut(&path).unwrap().0 = 0;
        let listed = catalog.list().unwrap();
        assert_eq!(listed[0].overview().description.as_deref(), Some("first"));
    }

    fn entry(
        id: &str,
        description: Option<&str>,
        cwd: &str,
        mtime_ms: u128,
        span: Option<(i64, i64)>,
        calls: i64,
    ) -> SessionEntry {
        SessionEntry {
            id: id.parse().unwrap(),
            mtime_ms,
            path: PathBuf::new(),
            overview: Overview {
                start_ms: span.map(|(start, _)| start),
                end_ms: span.map(|(_, end)| end),
                calls,
                cwd: Some(cwd.to_owned()),
                description: description.map(str::to_owned),
                ..Overview::default()
            },
            error: None,
        }
    }

    fn a() -> SessionEntry {
        entry(
            "abc12345",
            Some("Fix the failing commit hook"),
            "/work/ichnos",
            1000,
            Some((0, 125_000)),
            4,
        )
    }

    fn b() -> SessionEntry {
        entry("zzz99999", None, "/work/trees/1", 500, None, 0)
    }

    #[test]
    fn a_query_matches_when_every_word_is_in_the_description_folder_or_id() {
        assert!(a().matches("fix hook"));
        assert!(a().matches("ICHNOS commit"));
        assert!(a().matches("abc123"));
        assert!(!a().matches("fix banana"));
        assert!(b().matches("trees"));
        assert!(!b().matches("fix"));
    }

    #[test]
    fn an_empty_query_keeps_every_session_in_order() {
        let sessions = [a(), b()];
        let ids = |query| {
            filter_sessions(&sessions, query)
                .map(|s| s.id().as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("  "), ["abc12345", "zzz99999"]);
        assert_eq!(ids("zzz"), ["zzz99999"]);
    }

    #[test]
    fn a_folder_shows_its_last_part_with_the_parent_for_short_names() {
        assert_eq!(project_of("/work/ichnos"), "ichnos");
        assert_eq!(project_of("/work/trees/1"), "trees/1");
        assert_eq!(project_of(""), "");
    }

    #[test]
    fn spans_read_as_seconds_minutes_or_hours_and_minutes() {
        assert_eq!(span_label(45_000), "45s");
        assert_eq!(span_label(12 * 60_000), "12m");
        assert_eq!(span_label((2 * 60 + 5) * 60_000), "2h 05m");
    }

    #[test]
    fn facts_list_the_folder_length_and_calls_that_are_known() {
        let facts = a().facts();
        assert_eq!(facts[0], "ichnos");
        assert!(facts.contains(&"2m".to_owned()));
        assert!(facts.contains(&"4 calls".to_owned()));
        assert!(
            !b().facts()
                .iter()
                .any(|fact| fact.contains("call") || fact.ends_with('m') && fact.len() <= 3)
        );
    }

    #[test]
    fn a_session_is_recent_while_it_changed_in_the_last_half_minute() {
        assert!(entry("x", None, "", 90_000, None, 0).is_recent(100_000));
        assert!(!entry("x", None, "", 10_000, None, 0).is_recent(100_000));
    }
}
