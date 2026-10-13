//! `ichnos-app`: Ichnos's native viewer. Shows a coding agent's session as a live timeline,
//! reading the session databases that `ichnos log` and `ichnos hook` write.
//!
//! `ichnos open` starts it; it can also be run directly:
//! `ichnos-app [<session>] [--view tree|graph|table] [--dir <dir>] [--no-summaries] [--no-search]
//! [--summarizer <command>]`.

#![expect(
    clippy::too_many_lines,
    reason = "GPUI element trees read top to bottom; splitting them scatters one layout over helpers"
)]

mod live;
mod llm;
mod prefs;
mod present;
mod settings;
#[cfg(feature = "screenshots")]
mod shots;
mod summarizer;
mod summarizers;
mod theme;
mod view_state;
mod viewer;
mod zoom;

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use gpui_kit::{AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use ichnos_core::instance::{self, Command, Listener};
use ichnos_core::store::SessionId;
use ichnos_search::Search;

use crate::prefs::Prefs;
use crate::summarizer::Summarizer;
use crate::view_state::View;
use crate::viewer::{Options, Viewer};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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
    /// instructions in `$ICHNOS_SYSTEM_PROMPT`, and prints the reply [default: the one chosen in
    /// Settings, else Haiku through `claude -p`].
    #[arg(long, env = "ICHNOS_SUMMARIZER")]
    summarizer: Option<String>,
    /// Don't index sessions for search by meaning in the session switcher.
    #[arg(long)]
    no_search: bool,
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
    #[cfg(feature = "screenshots")]
    if let Some(out) = std::env::var_os("ICHNOS_SHOTS") {
        shots::run(out.as_ref(), &data_dir, cli.session.as_ref());
        return ExitCode::SUCCESS;
    }
    // Fill the cached login-shell `PATH` off the UI thread; the first read takes about half a
    // second.
    std::thread::spawn(summarizers::login_path);
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
    let search = if cli.no_search {
        None
    } else {
        match Search::start(sessions_dir.clone(), data_dir.clone()) {
            Ok(search) => Some(search),
            Err(err) => {
                eprintln!("search by meaning is off: {err}");
                None
            }
        }
    };
    let summarizer = summarizers::initial(
        cli.summarizer.as_deref(),
        Prefs::load(&data_dir).summarizer.as_deref(),
    );
    let summaries = if cli.no_summaries {
        None
    } else {
        match Summarizer::start(sessions_dir, data_dir.clone(), summarizer.model()) {
            Ok(summarizer) => Some(summarizer),
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
        summarizer,
        summarizers: summarizers::choices(cli.summarizer.as_deref()),
        probe: summarizers::probe,
        search,
        listener,
    };

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::apply_appearance(cx.window_appearance(), cx);
            viewer::bind_keys(cx);
            viewer::set_menus(cx);
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
