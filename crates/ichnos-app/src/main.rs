//! `ichnos-app`: Ichnos's native viewer. Shows a coding agent's session as a live timeline,
//! reading the session databases that `ichnos log` and `ichnos hook` write.
//!
//! `ichnos open` starts it; it can also be run directly:
//! `ichnos-app [<session>] [--view tree|graph|table] [--dir <dir>] [--no-summaries]
//! [--summarizer <command>]`.

#![expect(
    clippy::too_many_lines,
    reason = "GPUI element trees read top to bottom; splitting them scatters one layout over helpers"
)]

mod live;
mod llm;
mod present;
mod summarizer;
mod summary_host;
mod theme;
mod view_state;
mod viewer;
mod zoom;

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::llm::Llm;
use clap::Parser;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use ichnos_core::instance::{self, Command, Listener};
use ichnos_core::store::SessionId;

use crate::summary_host::SummaryHost;
use crate::view_state::View;
use crate::viewer::{Options, Viewer};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The summarizer used unless `--summarizer` names another: Haiku through the Claude Code CLI,
/// with no tools, no MCP servers, hooks disabled and only user settings, so the summarizing
/// session never shows up in the logs itself. The system prompt replaces Claude Code's own.
const DEFAULT_SUMMARIZER: &str = concat!(
    "MAX_THINKING_TOKENS=0 claude -p --model haiku --tools '' --strict-mcp-config ",
    "--setting-sources user --settings '{\"disableAllHooks\":true,\"alwaysThinkingEnabled\":false}' ",
    "--no-session-persistence --disable-slash-commands ",
    "--system-prompt \"$ICHNOS_SYSTEM_PROMPT\" --output-format text",
);

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Session to show; the window stays on it [default: follow the newest session].
    session: Option<SessionId>,
    /// View to open on: tree, graph or table.
    #[arg(long, default_value_t = View::Tree)]
    view: View,
    /// Data directory; sessions are in `<dir>/sessions` [default: ~/.ichnos].
    #[arg(long, env = "ICHNOS_DIR")]
    dir: Option<PathBuf>,
    /// Don't summarize entries.
    #[arg(long)]
    no_summaries: bool,
    /// Command line that writes the one-line summaries: it gets the prompt on stdin and the
    /// instructions in `$ICHNOS_SYSTEM_PROMPT`, and prints the reply [default: Haiku through
    /// `claude -p`].
    #[arg(long, env = "ICHNOS_SUMMARIZER")]
    summarizer: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(data_dir) = cli.dir.or_else(ichnos_core::default_data_dir) else {
        eprintln!("no data directory: set HOME, --dir or ICHNOS_DIR");
        return ExitCode::FAILURE;
    };
    let sessions_dir = data_dir.join(ichnos_core::SESSIONS_DIR);
    if let Err(err) = std::fs::create_dir_all(&sessions_dir) {
        eprintln!("could not create {}: {err}", sessions_dir.display());
        return ExitCode::FAILURE;
    }
    // One app per data directory: a second launch hands its request to the first.
    let socket = instance::socket_path(&data_dir);
    let listener = match Listener::bind(&socket) {
        Ok(listener) => Some(listener),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            let show = Command::Show {
                session: cli.session,
                view: Some(cli.view),
            };
            return match instance::send(&socket, &show) {
                Ok(_) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("could not reach the running ichnos-app: {err}");
                    ExitCode::FAILURE
                }
            };
        }
        Err(err) => {
            eprintln!("ichnos open can't reach this window: {err}");
            None
        }
    };
    let summaries = if cli.no_summaries {
        None
    } else {
        let llm = Llm::new(cli.summarizer.as_deref().unwrap_or(DEFAULT_SUMMARIZER));
        match SummaryHost::start(sessions_dir, data_dir.clone(), llm) {
            Ok(host) => Some(host),
            Err(err) => {
                eprintln!("summaries are off: {err}");
                None
            }
        }
    };
    let options = Options {
        data_dir,
        session: cli.session,
        view: cli.view,
        summaries,
        listener,
    };

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        viewer::bind_keys(cx);
        let bounds = Bounds::centered(None, size(px(1400.), px(900.)), cx);
        let window = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Ichnos".into()),
                    ..TitlebarOptions::default()
                }),
                ..WindowOptions::default()
            },
            cx,
            |window, cx| cx.new(|cx| Viewer::new(options, window, cx)),
        );
        match window {
            Ok(_) => cx.activate(true),
            Err(err) => {
                eprintln!("could not open the window: {err}");
                cx.quit();
            }
        }
    });
    ExitCode::SUCCESS
}
