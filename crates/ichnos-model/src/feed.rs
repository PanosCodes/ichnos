//! Everything a viewer sees for one session, as it happens: events (model messages included) and
//! summaries.
//!
//! A [`Feed`] keeps one stream's cursors (the last event and summary it handed out). Each
//! [`Feed::poll`] returns what's new as plain [`Item`]s for the viewer to apply to its
//! [`SessionData`](crate::session_data::SessionData). The database is the only source.

use std::path::Path;

use ichnos_core::store::{Row, Store, StoreError};

/// One new thing for the viewer.
#[derive(Debug, Clone)]
pub enum Item {
    /// A logged event; its row id is the resume cursor.
    Event(Row),
    /// A summary as `{"event_id", "summary"}` JSON.
    Summary(Row),
}

/// One session's feed for one viewer.
#[derive(Debug)]
pub struct Feed {
    store: Store,
    last_event_id: i64,
    /// Summaries are always sent in full to a new feed; the viewer keeps them in a map.
    last_summary_id: i64,
}

impl Feed {
    /// A feed of the session in the database at `db`, starting after event row `resume_after`
    /// (0 for the whole session).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Open`] when the database can't be opened.
    pub fn open(db: &Path, resume_after: i64) -> Result<Self, StoreError> {
        Ok(Self {
            store: Store::open(db)?,
            last_event_id: resume_after,
            last_summary_id: 0,
        })
    }

    /// Everything new since the last poll: events first, then summaries.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] when the database can't be read; the next poll retries.
    pub fn poll(&mut self) -> Result<Vec<Item>, StoreError> {
        let mut items = Vec::new();
        for row in self.store.events_after(self.last_event_id)? {
            self.last_event_id = row.id;
            items.push(Item::Event(row));
        }
        for row in self.store.summaries_after(self.last_summary_id)? {
            self.last_summary_id = row.id;
            items.push(Item::Summary(row));
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ichnos_core::store::NewEvent;

    fn add(store: &Store, json: &str) -> i64 {
        store
            .add_event(&NewEvent {
                epoch_ms: 1,
                event: "Stop",
                tool_use_id: None,
                json,
            })
            .unwrap()
    }

    #[test]
    fn sends_events_and_summaries_once_each() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let mut writer = Store::open(&db).unwrap();
        let first_id = add(&writer, "{\"n\":1}");
        writer
            .add_summaries(&[(first_id, "Started.".into())])
            .unwrap();

        let mut feed = Feed::open(&db, 0).unwrap();
        let items = feed.poll().unwrap();
        let [Item::Event(event), Item::Summary(summary)] = items.as_slice() else {
            panic!("expected event, summary; got {items:?}");
        };
        assert_eq!((event.id, event.json.as_str()), (first_id, "{\"n\":1}"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&summary.json).unwrap(),
            serde_json::json!({"event_id": first_id, "summary": "Started."})
        );
        assert!(feed.poll().unwrap().is_empty(), "nothing is sent twice");

        let next_id = add(&writer, "{\"n\":2}");
        let items = feed.poll().unwrap();
        assert!(matches!(items.as_slice(), [Item::Event(row)] if row.id == next_id));
    }

    #[test]
    fn resumes_after_the_given_event() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let writer = Store::open(&db).unwrap();
        let first = add(&writer, "{}");
        let second = add(&writer, "{}");

        let mut feed = Feed::open(&db, first).unwrap();
        let items = feed.poll().unwrap();
        assert!(matches!(items.as_slice(), [Item::Event(row)] if row.id == second));
    }
}
