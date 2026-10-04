//! One indexing step: embed what is new in a session and store it.

use std::path::Path;

use ichnos_core::store::{SessionId, Store};

use crate::SearchError;
use crate::docs::docs_from;
use crate::embedder::Embedder;
use crate::index::{Cursor, Index};

/// Embed and store everything in `session` after its cursor; returns how many docs were added.
///
/// # Errors
///
/// Returns the [`SearchError`] when the session can't be read, the text can't be embedded or the
/// index can't be written. The cursor doesn't move then, so the next step tries again.
pub(crate) fn index_session(
    sessions_dir: &Path,
    session: &SessionId,
    index: &mut Index,
    embedder: &Embedder,
) -> Result<usize, SearchError> {
    let cursor = index.cursor(session)?;
    let store = Store::open(&session.db_path(sessions_dir))?;
    let events = store.events_after(cursor.events)?;
    let summaries = store.summaries_after(cursor.summaries)?;
    if events.is_empty() && summaries.is_empty() {
        return Ok(0);
    }
    let next = Cursor {
        events: events.last().map_or(cursor.events, |row| row.id),
        summaries: summaries.last().map_or(cursor.summaries, |row| row.id),
    };
    let docs = docs_from(&events, &summaries);
    let texts: Vec<String> = docs.iter().map(|doc| doc.text.clone()).collect();
    let vectors = if texts.is_empty() {
        Vec::new()
    } else {
        embedder.embed(&texts)?
    };
    index.add(session, &docs, &vectors, next)
}

#[cfg(test)]
mod tests {
    use ichnos_core::store::NewEvent;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::embedder::MODEL;

    fn prompt(store: &Store, text: &str) -> i64 {
        let json = json!({"event": "UserPromptSubmit", "epoch_ms": 1, "prompt": text}).to_string();
        store
            .add_event(&NewEvent {
                epoch_ms: 1,
                event: "UserPromptSubmit",
                tool_use_id: None,
                json: &json,
            })
            .unwrap()
    }

    #[test]
    fn each_step_adds_only_what_is_new_including_late_summaries() {
        let dir = TempDir::new().unwrap();
        let session: SessionId = "s".parse().unwrap();
        let mut store = Store::open(&session.db_path(dir.path())).unwrap();
        let mut index = Index::open(&dir.path().join("index.db"), MODEL).unwrap();
        let embedder = Embedder::fake();
        let step = |index: &mut Index| index_session(dir.path(), &session, index, &embedder);

        let first = prompt(&store, "fix the login bug");
        assert_eq!(step(&mut index).unwrap(), 1);
        assert_eq!(step(&mut index).unwrap(), 0);

        prompt(&store, "add a search box");
        store
            .add_summaries(&[(first, "Fixed the login bug.".to_owned())])
            .unwrap();
        assert_eq!(step(&mut index).unwrap(), 2);
        assert_eq!(step(&mut index).unwrap(), 0);
    }
}
