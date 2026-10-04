//! Registers the [`sqlite-vec`](https://github.com/asg017/sqlite-vec) extension with `SQLite`, so
//! connections get the `vec0` virtual table.
//!
//! This is the only Ichnos crate that may use `unsafe`: the extension can only be registered
//! through `SQLite`'s C API. Everything else stays under the workspace's `forbid(unsafe_code)`.

use std::sync::OnceLock;

use rusqlite::auto_extension::{RawAutoExtension, register_auto_extension};

/// Register `sqlite-vec` so every connection opened afterwards in this process has `vec0`.
///
/// Safe to call more than once; later calls return the first call's outcome.
///
/// # Errors
///
/// Returns `SQLite`'s message when it rejects the registration.
pub fn register() -> Result<(), String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| {
            // SAFETY: the `sqlite-vec` crate declares `sqlite3_vec_init` as `extern "C" fn()`, but
            // the C symbol is an `SQLite` extension entry point, which is the signature
            // `RawAutoExtension` describes. This is sqlite-vec's documented rusqlite registration.
            #[expect(
                unsafe_code,
                reason = "sqlite-vec can only be registered through the C API"
            )]
            unsafe {
                let init = std::mem::transmute::<unsafe extern "C" fn(), RawAutoExtension>(
                    sqlite_vec::sqlite3_vec_init,
                );
                register_auto_extension(init).map_err(|err| err.to_string())
            }
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    #[test]
    fn connections_opened_after_registering_have_vec0() {
        super::register().unwrap();
        super::register().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        let version: String = conn
            .query_row("SELECT vec_version()", [], |row| row.get(0))
            .unwrap();
        assert!(version.starts_with('v'));
    }
}
