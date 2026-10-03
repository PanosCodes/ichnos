//! `ichnos open`: show a session in the viewer app, starting `ichnos-app` in the background
//! when it isn't running.
//!
//! The hook runs this as a detached process on `SessionStart` (see [`spawn`]), without delaying the
//! hook. It starts the app on the new session only when no app is running; a running app lists
//! the new session already (and switches to it when following the newest).
//!
//! `ichnos stop` ([`stop`]) asks the running app to quit. Both talk to the app over its socket
//! (see [`ichnos_core::instance`]).

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ichnos_core::instance::{self, Delivery, View};
use ichnos_core::store::SessionId;
use thiserror::Error;

/// The viewer app's executable, looked up next to `ichnos` and then on `PATH`.
const APP_EXE: &str = "ichnos-app";
/// How long a freshly started app gets to answer on its socket.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a quitting app gets to stop answering.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// How often to check on a starting or stopping app.
const POLL: Duration = Duration::from_millis(100);
/// Where a background app writes its log, in the data directory.
const APP_LOG: &str = "app.log";

/// Why the viewer could not be opened.
#[derive(Debug, Error)]
pub enum OpenError {
    #[error("could not find the ichnos executable")]
    Exe(#[source] io::Error),

    #[error(
        "could not start `{APP_EXE}`; install it with `cargo install --path crates/ichnos-app`"
    )]
    Spawn(#[source] io::Error),

    #[error("the running `{APP_EXE}` did not take the request")]
    Socket(#[source] io::Error),

    #[error("`{APP_EXE}` did not start; see {}", log.display())]
    NotReady { log: PathBuf },
}

/// Why the viewer could not be stopped.
#[derive(Debug, Error)]
pub enum StopError {
    #[error("the running `{APP_EXE}` did not take the request")]
    Socket(#[source] io::Error),

    #[error("`{APP_EXE}` is still running after being asked to quit")]
    StillRunning,
}

/// What `ichnos open` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// The running app shows the session now.
    Shown,
    /// The app was started on the session.
    Started,
    /// An app was running, so nothing was done (`--unless-open`).
    AlreadyOpen,
}

/// What to open and how.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenArgs<'a> {
    /// The session to pin; `None` follows the newest.
    pub session: Option<&'a SessionId>,
    /// The view to show.
    pub view: Option<View>,
    /// Do nothing when an app is running (how the hook runs it).
    pub unless_open: bool,
    /// Start the app without summaries.
    pub no_summaries: bool,
    /// The command line the app's summarizer runs; `None` keeps the app's default.
    pub summarizer: Option<&'a str>,
}

/// Show `args.session` in the viewer app serving data directory `dir`, starting it if needed.
///
/// # Errors
///
/// Returns an [`OpenError`] when the app can't be reached, found or started.
pub fn run(dir: &Path, args: OpenArgs<'_>) -> Result<Opened, OpenError> {
    let socket = instance::socket_path(dir);
    let command = if args.unless_open {
        instance::Command::Ping
    } else {
        instance::Command::Show {
            session: args.session.cloned(),
            view: args.view,
        }
    };
    match instance::send(&socket, &command).map_err(OpenError::Socket)? {
        Delivery::Delivered if args.unless_open => Ok(Opened::AlreadyOpen),
        Delivery::Delivered => Ok(Opened::Shown),
        Delivery::NoApp => {
            start_app(dir, &socket, args)?;
            Ok(Opened::Started)
        }
    }
}

/// Ask the app serving data directory `dir` to quit and wait until it has. Returns `false` when
/// no app was running.
///
/// # Errors
///
/// Returns a [`StopError`] when the app doesn't take the request or keeps running.
pub fn stop(dir: &Path) -> Result<bool, StopError> {
    let socket = instance::socket_path(dir);
    match instance::send(&socket, &instance::Command::Quit).map_err(StopError::Socket)? {
        Delivery::NoApp => Ok(false),
        Delivery::Delivered if wait_until(STOP_TIMEOUT, || !answers(&socket)) => Ok(true),
        Delivery::Delivered => Err(StopError::StillRunning),
    }
}

/// Start `ichnos open --unless-open <session>` in the background without waiting for it.
///
/// It runs detached with `--dir <dir>`, and its errors are appended to `error_log`.
///
/// # Errors
///
/// Returns an [`OpenError`] when the executable can't be found or started.
pub fn spawn(dir: &Path, session: &SessionId, error_log: &Path) -> Result<(), OpenError> {
    let exe = std::env::current_exe().map_err(OpenError::Exe)?;
    let mut command = Command::new(exe);
    command
        .arg("--dir")
        .arg(dir)
        .args(["open", "--unless-open", session.as_str()]);
    detached(&mut command, error_log)
        .spawn()
        .map_err(OpenError::Spawn)?;
    Ok(())
}

/// Start the app in the background and wait until it answers on `socket`.
fn start_app(dir: &Path, socket: &Path, args: OpenArgs<'_>) -> Result<(), OpenError> {
    let log = dir.join(APP_LOG);
    let mut command = Command::new(app_exe());
    command.arg("--dir").arg(dir);
    if let Some(session) = args.session {
        command.arg(session.as_str());
    }
    if let Some(view) = args.view {
        command.args(["--view", view.as_str()]);
    }
    if args.no_summaries {
        command.arg("--no-summaries");
    }
    if let Some(summarizer) = args.summarizer {
        command.args(["--summarizer", summarizer]);
    }
    let mut child = detached(&mut command, &log)
        .spawn()
        .map_err(OpenError::Spawn)?;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if answers(socket) {
            return Ok(());
        }
        // Another `open` may have started an app first; ours then hands over and exits.
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        std::thread::sleep(POLL);
    }
    if answers(socket) {
        Ok(())
    } else {
        Err(OpenError::NotReady { log })
    }
}

/// The app's executable: next to this one when installed together, else from `PATH`.
fn app_exe() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name(APP_EXE))
        .filter(|exe| exe.exists())
        .unwrap_or_else(|| PathBuf::from(APP_EXE))
}

/// Whether an app answers on `socket`.
fn answers(socket: &Path) -> bool {
    matches!(
        instance::send(socket, &instance::Command::Ping),
        Ok(Delivery::Delivered)
    )
}

/// Check `done` every [`POLL`] until it holds or `timeout` passes, and say whether it held.
fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// Set `command` up to run in the background, with stderr appended to `log`.
///
/// It gets no stdin or stdout and its own process group, so it outlives the caller and Claude
/// Code doesn't wait on it.
fn detached<'a>(command: &'a mut Command, log: &Path) -> &'a mut Command {
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_or_else(|_| Stdio::null(), Stdio::from);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    command
}

#[cfg(test)]
mod tests {
    use ichnos_core::instance::{Command as AppCommand, Listener};

    use super::*;

    #[test]
    fn a_running_app_is_told_to_show_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let app = Listener::bind(&instance::socket_path(dir.path())).unwrap();
        let session: SessionId = "s1".parse().unwrap();
        let args = OpenArgs {
            session: Some(&session),
            view: Some(View::Graph),
            ..OpenArgs::default()
        };
        assert_eq!(run(dir.path(), args).unwrap(), Opened::Shown);
        assert!(wait_until(Duration::from_secs(2), || matches!(
            app.try_recv(),
            Some(AppCommand::Show {
                session: Some(_),
                view: Some(View::Graph)
            })
        )));
    }

    #[test]
    fn the_hook_leaves_a_running_app_alone() {
        let dir = tempfile::tempdir().unwrap();
        let app = Listener::bind(&instance::socket_path(dir.path())).unwrap();
        let args = OpenArgs {
            unless_open: true,
            ..OpenArgs::default()
        };
        assert_eq!(run(dir.path(), args).unwrap(), Opened::AlreadyOpen);
        assert!(wait_until(Duration::from_secs(2), || app.try_recv()
            == Some(AppCommand::Ping)));
    }

    #[test]
    fn stopping_without_an_app_says_so() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!stop(dir.path()).unwrap());
    }

    #[test]
    fn waiting_gives_up_after_the_timeout() {
        assert!(!wait_until(Duration::from_millis(150), || false));
        assert!(wait_until(Duration::ZERO, || true));
    }
}
