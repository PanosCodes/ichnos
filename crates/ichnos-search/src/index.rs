//! The search index, `~/.ichnos/index.db`: indexed text and its vectors for every session.
//!
//! The only module that knows the index's SQL; the schema is in `index.sql` at this crate's root.

use std::path::Path;
use std::time::Duration;

use ichnos_core::store::SessionId;
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::SearchError;
use crate::docs::Doc;
use crate::rank::SessionMatch;

/// How long a write waits for another connection's lock before failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The current schema.
const SCHEMA: &str = include_str!("../index.sql");

/// Statements that empty the index, run before [`SCHEMA`] when the model changes.
const DROP_ALL: &str = "DROP TABLE IF EXISTS vectors;
DROP TABLE IF EXISTS docs;
DROP TABLE IF EXISTS cursors;
DROP TABLE IF EXISTS meta;";

/// How far a session has been indexed: the last event and summary row ids read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Cursor {
    pub events: i64,
    pub summaries: i64,
}

/// One connection to the index.
#[derive(Debug)]
pub(crate) struct Index {
    conn: Connection,
}

impl Index {
    /// Open or create the index at `path` for vectors made by `model`. An index made by another
    /// model is emptied, so every session is embedded again.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::Vec`] when `sqlite-vec` can't be registered, or
    /// [`SearchError::Index`] when the database can't be opened or set up.
    pub(crate) fn open(path: &Path, model: &str) -> Result<Self, SearchError> {
        ichnos_vec::register().map_err(SearchError::Vec)?;
        let conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        let stored: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = 'model'", [], |row| {
                row.get(0)
            })
            .optional()?;
        if stored.as_deref() != Some(model) {
            if stored.is_some() {
                conn.execute_batch(DROP_ALL)?;
                conn.execute_batch(SCHEMA)?;
            }
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('model', ?1)",
                params![model],
            )?;
        }
        Ok(Self { conn })
    }

    /// How far `session` has been indexed; zero for a session not seen yet.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::Index`] when the query fails.
    pub(crate) fn cursor(&self, session: &SessionId) -> Result<Cursor, SearchError> {
        let cursor = self
            .conn
            .query_row(
                "SELECT events, summaries FROM cursors WHERE session_id = ?1",
                params![session.as_str()],
                |row| {
                    Ok(Cursor {
                        events: row.get(0)?,
                        summaries: row.get(1)?,
                    })
                },
            )
            .optional()?;
        Ok(cursor.unwrap_or_default())
    }

    /// Store `docs` with their `vectors` (one per doc, in order) and move the cursor of `session`
    /// to `cursor`, all in one transaction. A doc already stored is skipped with its vector.
    /// Returns how many docs were added.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::Index`] when a write fails; nothing is stored then.
    ///
    /// # Panics
    ///
    /// Panics when `docs` and `vectors` differ in length, which is a bug in the caller.
    pub(crate) fn add(
        &mut self,
        session: &SessionId,
        docs: &[Doc],
        vectors: &[Vec<f32>],
        cursor: Cursor,
    ) -> Result<usize, SearchError> {
        assert_eq!(docs.len(), vectors.len(), "one vector per doc");
        let tx = self.conn.transaction()?;
        let mut added = 0;
        {
            let mut add_doc = tx.prepare_cached(
                "INSERT INTO docs (session_id, event_id, kind, text) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT DO NOTHING RETURNING id",
            )?;
            let mut add_vector =
                tx.prepare_cached("INSERT INTO vectors (rowid, embedding) VALUES (?1, ?2)")?;
            for (doc, vector) in docs.iter().zip(vectors) {
                let id: Option<i64> = add_doc
                    .query_row(
                        params![session.as_str(), doc.event_id, doc.kind.as_str(), doc.text],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(id) = id {
                    add_vector.execute(params![id, bytes(vector)])?;
                    added += 1;
                }
            }
            tx.execute(
                "INSERT INTO cursors (session_id, events, summaries) VALUES (?1, ?2, ?3)
                 ON CONFLICT (session_id) DO UPDATE
                 SET events = excluded.events, summaries = excluded.summaries",
                params![session.as_str(), cursor.events, cursor.summaries],
            )?;
        }
        tx.commit()?;
        Ok(added)
    }

    /// The `k` docs closest to `query`, closest first.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::Index`] when the query fails, e.g. for a vector of the wrong length.
    pub(crate) fn nearest(
        &self,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<SessionMatch>, SearchError> {
        // `vec0` applies `k` only when it is queried on its own, so the join is outside the CTE.
        let mut statement = self.conn.prepare_cached(
            "WITH knn AS (
                 SELECT rowid, distance FROM vectors WHERE embedding MATCH ?1 AND k = ?2
             )
             SELECT d.session_id, d.event_id, d.kind, d.text, knn.distance
             FROM knn JOIN docs d ON d.id = knn.rowid
             ORDER BY knn.distance",
        )?;
        let k = i64::try_from(k).unwrap_or(i64::MAX);
        let rows = statement.query_map(params![bytes(query), k], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })?;
        let mut found = Vec::new();
        for row in rows {
            let (session, event_id, kind, text, distance) = row?;
            // Rows this module didn't write are skipped rather than failing the whole search.
            let (Ok(session), Ok(kind)) = (session.parse(), kind.parse()) else {
                continue;
            };
            #[expect(
                clippy::cast_possible_truncation,
                reason = "distances are between 0 and 2"
            )]
            let distance = distance as f32;
            found.push(SessionMatch {
                session,
                event_id,
                kind,
                text,
                distance,
            });
        }
        Ok(found)
    }
}

/// `vector` as `sqlite-vec` reads a `float[]`: little-endian `f32`s.
fn bytes(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::docs::DocKind;
    use crate::embedder::{DIMENSIONS, Embedder, MODEL};

    fn session(id: &str) -> SessionId {
        id.parse().unwrap()
    }

    fn doc(event_id: i64, text: &str) -> Doc {
        Doc {
            event_id,
            kind: DocKind::Summary,
            text: text.to_owned(),
        }
    }

    fn add(index: &mut Index, id: &str, docs: &[Doc], cursor: Cursor) -> usize {
        let texts: Vec<String> = docs.iter().map(|doc| doc.text.clone()).collect();
        let vectors = Embedder::fake().embed(&texts).unwrap();
        index.add(&session(id), docs, &vectors, cursor).unwrap()
    }

    fn query(index: &Index, text: &str) -> Vec<String> {
        let vector = Embedder::fake().embed(&[text.to_owned()]).unwrap();
        index
            .nearest(&vector[0], 10)
            .unwrap()
            .into_iter()
            .map(|found| found.text)
            .collect()
    }

    #[test]
    fn the_nearest_docs_come_first() {
        let dir = TempDir::new().unwrap();
        let mut index = Index::open(&dir.path().join("index.db"), MODEL).unwrap();
        let docs = [
            doc(1, "listed files in docs"),
            doc(2, "fixed the login token bug"),
        ];
        add(&mut index, "a", &docs, Cursor::default());
        assert_eq!(
            query(&index, "login token"),
            ["fixed the login token bug", "listed files in docs"]
        );
    }

    #[test]
    fn a_doc_stored_twice_is_kept_once_and_the_cursor_moves() {
        let dir = TempDir::new().unwrap();
        let mut index = Index::open(&dir.path().join("index.db"), MODEL).unwrap();
        let cursor = Cursor {
            events: 4,
            summaries: 2,
        };
        assert_eq!(add(&mut index, "a", &[doc(1, "ran tests")], cursor), 1);
        assert_eq!(add(&mut index, "a", &[doc(1, "ran tests")], cursor), 0);
        assert_eq!(index.cursor(&session("a")).unwrap(), cursor);
        assert_eq!(index.cursor(&session("b")).unwrap(), Cursor::default());
        assert_eq!(query(&index, "ran tests").len(), 1);
    }

    #[test]
    fn another_model_empties_the_index() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.db");
        let mut index = Index::open(&path, MODEL).unwrap();
        add(
            &mut index,
            "a",
            &[doc(1, "ran tests")],
            Cursor {
                events: 3,
                summaries: 0,
            },
        );
        drop(index);

        let same = Index::open(&path, MODEL).unwrap();
        assert_eq!(query(&same, "ran tests").len(), 1);
        drop(same);

        let other = Index::open(&path, "other/model").unwrap();
        assert_eq!(query(&other, "ran tests"), Vec::<String>::new());
        assert_eq!(other.cursor(&session("a")).unwrap(), Cursor::default());
    }

    #[test]
    fn the_schema_holds_vectors_of_the_model_dimension() {
        let dir = TempDir::new().unwrap();
        let mut index = Index::open(&dir.path().join("index.db"), MODEL).unwrap();
        let vectors = [vec![0.5; DIMENSIONS]];
        index
            .add(&session("a"), &[doc(1, "x")], &vectors, Cursor::default())
            .unwrap();
    }
}
