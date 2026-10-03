//! What the Ichnos binaries share: the per-session SQLite store, the live feed that tails a
//! session, the session model, the sessions catalog and the Haiku summarizer.
//!
//! The `ichnos` CLI writes to the store from the Claude Code hook; a viewer reads it through
//! [`feed::Feed`] and [`catalog::Catalog`].

pub mod catalog;
pub mod error_log;
pub mod event;
pub mod store;
pub mod summarizer;
pub mod transcript;

/// Folder of session databases inside the data directory.
pub const SESSIONS_DIR: &str = "sessions";

/// Data directory under the home directory, used unless `--dir` or `ICHNOS_DIR` say otherwise.
pub const DEFAULT_DIR_IN_HOME: &str = ".ichnos";

/// The default data directory, `~/.ichnos`; `None` without a home directory.
#[must_use]
pub fn default_data_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(DEFAULT_DIR_IN_HOME))
}
