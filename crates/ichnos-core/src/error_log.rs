//! `hook-errors.log`: where failures go when there is nobody to tell, e.g. inside the hook or a
//! background worker. Writing to it never fails loudly.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;

/// File that hook and summarizer failures are appended to, in the data directory.
pub const ERROR_LOG: &str = "hook-errors.log";

/// Append a timestamped line to `<dir>/hook-errors.log`, ignoring any failure to do so.
pub fn append(dir: &Path, message: &str) {
    let line = format!("{} {message}\n", chrono::Utc::now().to_rfc3339());
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(ERROR_LOG))
    {
        // Nowhere left to report a failed write.
        let _ = file.write_all(line.as_bytes());
    }
}
