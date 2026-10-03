//! Runs the summarizer on a tokio runtime of its own, so it never shares an executor with GPUI.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use crate::llm::Llm;
use crate::summarizer::{Summarizer, Watch};
use ichnos_core::store::SessionId;

/// A handle to the summarizer thread; it runs until the app exits.
#[derive(Debug, Clone)]
pub struct SummaryHost {
    summarizer: Summarizer,
}

impl SummaryHost {
    /// Start the summarizer for the session databases in `sessions_dir`, asking `llm` for the
    /// sentences; failures go to `<data_dir>/hook-errors.log`.
    ///
    /// # Errors
    ///
    /// Returns the [`io::Error`] when the thread or its runtime can't start.
    pub fn start(sessions_dir: PathBuf, data_dir: PathBuf, llm: Llm) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("ichnos-summarizer".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        // The receiver turns a dropped sender into an error.
                        drop(sender);
                        ichnos_core::error_log::append(
                            &data_dir,
                            &format!("summarizer runtime: {err:?}"),
                        );
                        return;
                    }
                };
                runtime.block_on(async move {
                    let summarizer = Summarizer::start(sessions_dir, data_dir, llm);
                    if sender.send(summarizer).is_ok() {
                        std::future::pending::<()>().await;
                    }
                });
            })?;
        let summarizer = receiver
            .recv()
            .map_err(|_| io::Error::other("the summarizer runtime did not start"))?;
        Ok(Self { summarizer })
    }

    /// Summarize `session` until the returned guard is dropped.
    #[must_use]
    pub fn watch(&self, session: SessionId) -> Watch {
        self.summarizer.watch(session)
    }
}
