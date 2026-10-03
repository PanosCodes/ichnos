//! One session followed live: its feed, the data received so far and the connection state.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use crate::summarizer::Watch;
use ichnos_core::store::{SessionId, StoreError};
use ichnos_model::feed::{Feed, Item};
use ichnos_model::session_data::SessionData;

/// Whether the session's database is being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    /// Not read yet.
    Connecting,
    /// Read on the last poll.
    Live,
    /// The last poll failed; the next one retries.
    Retry,
}

/// A feed shared with the background executor that polls it.
pub type SharedFeed = Arc<Mutex<Feed>>;

/// One session's live data.
#[derive(Debug)]
pub struct LiveSession {
    id: SessionId,
    feed: Option<SharedFeed>,
    data: SessionData,
    connection: Connection,
    /// Bumped whenever data arrives.
    version: u64,
    /// Keeps the session summarized while it's shown.
    _watch: Option<Watch>,
}

impl LiveSession {
    /// Follow session `id` from its database in the sessions folder `dir`.
    pub fn open(dir: &Path, id: SessionId, watch: Option<Watch>) -> Self {
        let mut live = Self {
            feed: None,
            data: SessionData::default(),
            connection: Connection::Connecting,
            version: 0,
            _watch: watch,
            id,
        };
        live.reopen(dir);
        live
    }

    /// Open the feed if it isn't open yet, e.g. because the database didn't exist.
    pub fn reopen(&mut self, dir: &Path) {
        if self.feed.is_none() {
            match Feed::open(&self.id.db_path(dir), 0) {
                Ok(feed) => self.feed = Some(Arc::new(Mutex::new(feed))),
                Err(_) => self.connection = Connection::Retry,
            }
        }
    }

    /// The session's id.
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// The feed to poll, once open.
    pub fn feed(&self) -> Option<SharedFeed> {
        self.feed.clone()
    }

    /// The data received so far.
    pub fn data(&self) -> &SessionData {
        &self.data
    }

    /// Whether the database is being read.
    pub fn connection(&self) -> Connection {
        self.connection
    }

    /// Bumped whenever data arrives.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Take in one poll's result; `true` when data arrived.
    pub fn apply(&mut self, polled: Result<Vec<Item>, StoreError>) -> bool {
        let Ok(items) = polled else {
            self.connection = Connection::Retry;
            return false;
        };
        self.connection = Connection::Live;
        let changed = !items.is_empty();
        for item in items {
            // A record that can't be parsed is skipped; the rest of the session still shows.
            let _ = self.data.apply(item);
        }
        if changed {
            self.version += 1;
        }
        changed
    }
}

/// Poll `feed` once, blocking; run it on a background thread.
pub fn poll(feed: &SharedFeed) -> Result<Vec<Item>, StoreError> {
    feed.lock().unwrap_or_else(PoisonError::into_inner).poll()
}
