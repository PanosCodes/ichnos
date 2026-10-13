//! What both Ichnos binaries share: the Ichnos event format and its one writer ([`log`]), the
//! per-session SQLite store, the sessions catalog, the app socket protocol and the debug account
//! format. It runs on every hook event, so it has no async runtime and no viewer logic; the
//! viewer's model lives in `ichnos-model`.
//!
//! Nothing here knows which coding agent produced a session. The `ichnos` CLI's adapters
//! translate an agent's own payloads into Ichnos events and hand them to [`log::Logger`]; the
//! viewer reads them back through `ichnos-model`'s feed and [`catalog::Catalog`].

pub mod catalog;
pub mod error_log;
pub mod event;
pub mod instance;
pub mod log;
pub mod store;

/// Folder of session databases inside the data directory.
pub const SESSIONS_DIR: &str = "sessions";

/// Data directory under the home directory, used unless `--dir` or `ICHNOS_DIR` say otherwise.
pub const DEFAULT_DIR_IN_HOME: &str = ".ichnos";

/// The default data directory, `~/.ichnos`; `None` without a home directory.
#[must_use]
pub fn default_data_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(DEFAULT_DIR_IN_HOME))
}

/// An epoch-millisecond moment in local time, formatted with `chrono`'s `format`; `None` when it
/// is out of range.
#[must_use]
pub fn local_time(ms: i64, format: &str) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|at| at.with_timezone(&chrono::Local).format(format).to_string())
}
