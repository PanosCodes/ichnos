//! Search Ichnos sessions by meaning, not only by their words.
//!
//! [`Search::start`] runs a background thread that embeds every session's prompts, errors of
//! failed calls and summaries with a `Model2Vec` model, and keeps them in a `sqlite-vec` index at
//! `<data_dir>/index.db`. [`Search::query`] finds the sessions whose text is closest in meaning.

mod docs;
mod embedder;
mod index;
mod indexer;
mod rank;

use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::Duration;

use ichnos_core::catalog::Catalog;
use ichnos_core::store::{SessionId, StoreError};
use thiserror::Error;

#[doc(inline)]
pub use docs::DocKind;
#[doc(inline)]
pub use rank::SessionMatch;

use embedder::{Embedder, MODEL};
use index::Index;

/// The index file inside the data directory.
pub const INDEX_FILE: &str = "index.db";

/// Pause between passes over the sessions folder.
const INDEX_PAUSE: Duration = Duration::from_secs(2);

/// Docs read from the index per search, before keeping one per session.
const NEAREST_DOCS: usize = 50;

/// Sessions a search returns at most.
const MAX_RESULTS: usize = 8;

/// Why search failed.
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("the embedding model is still loading")]
    NotReady,

    #[error("could not load the embedding model")]
    Model(#[source] Box<dyn Error + Send + Sync>),

    #[error("the embedding model failed: {0}")]
    Embed(String),

    #[error("could not register sqlite-vec: {0}")]
    Vec(String),

    #[error("search index query failed")]
    Index(#[from] rusqlite::Error),

    #[error("could not read a session")]
    Store(#[from] StoreError),
}

/// Whether search can answer yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchState {
    /// The model is loading (or downloading, the first time).
    Loading,
    /// Searches work; sessions may still be being indexed.
    Ready,
    /// Search is off for this run, with the reason.
    Failed(String),
}

/// Search by meaning across sessions; cheap to clone, all clones share one indexer thread.
#[derive(Debug, Clone)]
pub struct Search {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    index_path: PathBuf,
    embedder: OnceLock<Embedder>,
    state: Mutex<SearchState>,
}

impl Search {
    /// Start the indexer thread over the session databases in `sessions_dir`, with the index at
    /// `<data_dir>/index.db`. It loads the model, then indexes whatever changed every
    /// [`INDEX_PAUSE`] until the process exits. Failures go to `<data_dir>/hook-errors.log`.
    ///
    /// # Errors
    ///
    /// Returns the [`io::Error`] when the thread can't start.
    pub fn start(sessions_dir: PathBuf, data_dir: PathBuf) -> io::Result<Self> {
        let search = Self {
            inner: Arc::new(Inner {
                index_path: data_dir.join(INDEX_FILE),
                embedder: OnceLock::new(),
                state: Mutex::new(SearchState::Loading),
            }),
        };
        let worker = search.clone();
        thread::Builder::new()
            .name("ichnos-search".to_owned())
            .spawn(move || worker.run(&sessions_dir, &data_dir))?;
        Ok(search)
    }

    /// Whether search can answer yet.
    #[must_use]
    pub fn state(&self) -> SearchState {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The sessions whose text is closest in meaning to `query`, closest first, one match each.
    /// Blocks on the database, so call it off the UI thread.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::NotReady`] while the model loads, or the error that stopped the
    /// search.
    pub fn query(&self, query: &str) -> Result<Vec<SessionMatch>, SearchError> {
        let embedder = self.inner.embedder.get().ok_or(SearchError::NotReady)?;
        let vectors = embedder.embed(&[query.to_owned()])?;
        let Some(vector) = vectors.first() else {
            return Ok(Vec::new());
        };
        let index = Index::open(&self.inner.index_path, MODEL)?;
        let mut found = rank::rank(index.nearest(vector, NEAREST_DOCS)?);
        found.truncate(MAX_RESULTS);
        Ok(found)
    }

    fn set_state(&self, state: SearchState) {
        *self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = state;
    }

    /// The indexer thread: load the model, then index changed sessions forever.
    fn run(&self, sessions_dir: &Path, data_dir: &Path) {
        let fail = |err: &SearchError| {
            let reason = match err.source() {
                Some(source) => format!("{err}: {source}"),
                None => err.to_string(),
            };
            ichnos_core::error_log::append(data_dir, &format!("search: {reason}"));
            self.set_state(SearchState::Failed(reason));
        };
        let embedder = match Embedder::load() {
            Ok(embedder) => embedder,
            Err(err) => return fail(&err),
        };
        let mut index = match Index::open(&self.inner.index_path, MODEL) {
            Ok(index) => index,
            Err(err) => return fail(&err),
        };
        let embedder = self.inner.embedder.get_or_init(|| embedder);
        self.set_state(SearchState::Ready);

        let mut catalog = Catalog::new(sessions_dir);
        // Session -> the modification time it had when it was last indexed.
        let mut indexed: Vec<(SessionId, u128)> = Vec::new();
        loop {
            for entry in catalog.list().unwrap_or_default() {
                let seen = indexed.iter().position(|(id, _)| id == entry.id());
                if seen.is_some_and(|at| indexed[at].1 == entry.mtime_ms()) {
                    continue;
                }
                match indexer::index_session(sessions_dir, entry.id(), &mut index, embedder) {
                    Ok(_) => match seen {
                        Some(at) => indexed[at].1 = entry.mtime_ms(),
                        None => indexed.push((entry.id().clone(), entry.mtime_ms())),
                    },
                    // Logged and tried again on the next pass.
                    Err(err) => ichnos_core::error_log::append(
                        data_dir,
                        &format!("search {}: {err:?}", entry.id()),
                    ),
                }
            }
            thread::sleep(INDEX_PAUSE);
        }
    }
}
