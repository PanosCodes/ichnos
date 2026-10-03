//! How `ichnos` talks to a running `ichnos-app`: one JSON command per line over the Unix
//! socket `<data dir>/app.sock`, answered with `ok`.
//!
//! The app binds a [`Listener`]; only one app can, so a second launch hands its request to the
//! first and exits. The CLI calls [`send`] and starts the app when nobody answers.

use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::store::SessionId;

/// The socket's file name in the data directory.
pub const SOCKET: &str = "app.sock";

/// How long a client waits to connect, write or read the answer.
const TIMEOUT: Duration = Duration::from_secs(2);

/// The reply to a command that arrived.
const OK: &str = "ok";

/// The longest socket path every Unix accepts, in bytes (`sun_path` is 104 bytes on macOS,
/// including the trailing NUL).
const MAX_SOCKET_PATH: usize = 100;

/// The socket of the app serving data directory `data_dir`: `<data_dir>/app.sock`, or, when that
/// path is too long for a Unix socket, `ichnos-<hash>.sock` in the user's temp directory.
#[must_use]
pub fn socket_path(data_dir: &Path) -> PathBuf {
    let socket = data_dir.join(SOCKET);
    if socket.as_os_str().len() <= MAX_SOCKET_PATH {
        return socket;
    }
    std::env::temp_dir().join(format!(
        "ichnos-{:016x}.sock",
        fnv1a(data_dir.as_os_str().as_encoded_bytes())
    ))
}

/// FNV-1a: a hash that stays the same across builds, so both binaries agree on the name.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

/// The viewer's main views.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    /// One row per prompt, expandable into calls and model steps.
    #[default]
    Tree,
    /// One card per agent.
    Graph,
    /// Every entry in time order.
    Table,
}

impl View {
    /// Every view, in toolbar order.
    pub const ALL: [Self; 3] = [Self::Tree, Self::Graph, Self::Table];

    /// The view's name: `tree`, `graph` or `table`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tree => "tree",
            Self::Graph => "graph",
            Self::Table => "table",
        }
    }

    /// The toolbar label: `Tree`, `Graph` or `Table`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Tree => "Tree",
            Self::Graph => "Graph",
            Self::Table => "Table",
        }
    }
}

impl fmt::Display for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for View {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|view| view.as_str() == name)
            .ok_or_else(|| format!("unknown view {name:?}: use tree, graph or table"))
    }
}

/// A request to the running app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Command {
    /// Bring the window forward on `session` (the newest, followed, when `None`).
    Show {
        /// The session to pin.
        session: Option<SessionId>,
        /// The view to switch to.
        view: Option<View>,
    },
    /// Do nothing; answers whether an app is running.
    Ping,
    /// Quit the app.
    Quit,
}

/// Whether a command reached an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The app received it.
    Delivered,
    /// No app is listening.
    NoApp,
}

/// Send `command` to the app on `socket`.
///
/// # Errors
///
/// Returns the [`io::Error`] when something listens but doesn't take the command.
pub fn send(socket: &Path, command: &Command) -> io::Result<Delivery> {
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return Ok(Delivery::NoApp);
    };
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut line = serde_json::to_string(command).map_err(io::Error::other)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    if reply.trim() == OK {
        Ok(Delivery::Delivered)
    } else {
        Err(io::Error::other(format!("unexpected reply {reply:?}")))
    }
}

/// The app's end of the socket; commands arrive on a channel. Dropping it removes the socket.
#[derive(Debug)]
pub struct Listener {
    path: PathBuf,
    commands: Receiver<Command>,
}

impl Listener {
    /// Listen on `socket`, replacing a socket left behind by an app that's gone.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::AddrInUse`] when another app answers on `socket`, or the
    /// [`io::Error`] when it can't be bound.
    pub fn bind(socket: &Path) -> io::Result<Self> {
        if UnixStream::connect(socket).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "another ichnos-app is running",
            ));
        }
        match std::fs::remove_file(socket) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
            _ => {}
        }
        let listener = UnixListener::bind(socket)?;
        let (sender, commands) = mpsc::channel();
        thread::Builder::new()
            .name("ichnos-instance".to_owned())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    if let Some(command) = read_command(stream)
                        && sender.send(command).is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            path: socket.to_owned(),
            commands,
        })
    }

    /// The next command that arrived, without waiting.
    #[must_use]
    pub fn try_recv(&self) -> Option<Command> {
        self.commands.try_recv().ok()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // A socket left behind is replaced by the next app's bind.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Read one command from `stream` and acknowledge it; `None` for anything else.
fn read_command(stream: UnixStream) -> Option<Command> {
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let command: Command = serde_json::from_str(&line).ok()?;
    let mut stream = reader.into_inner();
    stream.write_all(format!("{OK}\n").as_bytes()).ok()?;
    Some(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_reach_the_listener_and_are_acknowledged() {
        let dir = tempfile::tempdir().unwrap();
        let socket = socket_path(dir.path());
        let listener = Listener::bind(&socket).unwrap();
        let show = Command::Show {
            session: Some("s1".parse().unwrap()),
            view: Some(View::Graph),
        };
        assert_eq!(send(&socket, &show).unwrap(), Delivery::Delivered);
        assert_eq!(listener.commands.recv_timeout(TIMEOUT).unwrap(), show);
    }

    #[test]
    fn nobody_listening_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            send(&socket_path(dir.path()), &Command::Ping).unwrap(),
            Delivery::NoApp
        );
    }

    #[test]
    fn a_second_listener_is_refused_and_a_stale_socket_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let socket = socket_path(dir.path());
        let first = Listener::bind(&socket).unwrap();
        assert_eq!(
            Listener::bind(&socket).unwrap_err().kind(),
            io::ErrorKind::AddrInUse
        );
        drop(first);

        // A crashed app leaves its socket file; nobody answers on it.
        drop(UnixListener::bind(&socket).unwrap());
        assert!(socket.exists());
        assert!(Listener::bind(&socket).is_ok());
    }

    #[test]
    fn a_long_data_directory_gets_a_short_socket() {
        let short = Path::new("/home/me/.ichnos");
        assert_eq!(socket_path(short), short.join(SOCKET));
        let long = PathBuf::from("/").join("deep".repeat(40));
        let socket = socket_path(&long);
        assert!(
            socket.as_os_str().len() <= MAX_SOCKET_PATH,
            "{}",
            socket.display()
        );
        assert_eq!(
            socket,
            socket_path(&long),
            "the same directory gets the same socket"
        );
        assert_ne!(socket, socket_path(&long.join("other")));
    }

    #[test]
    fn views_parse_from_their_names() {
        assert_eq!("graph".parse::<View>(), Ok(View::Graph));
        assert!("grid".parse::<View>().is_err());
    }
}
