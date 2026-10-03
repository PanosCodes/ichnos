//! Logs every event of a coding agent's session to an SQLite database and opens a live viewer
//! on them.
//!
//! `ichnos log` reads Ichnos events (JSON lines, see `ichnos_core::log`) from stdin: the one
//! way in. `ichnos hook <adapter>` is the same for an agent whose hooks send their own payload;
//! the adapter translates it (`claude-code`, wired from the block in `hooks.json`). When an event
//! starts a session, both also run `ichnos open` in the background, which starts the viewer
//! unless it's running.
//! `ichnos open [<session>]` shows a session in the viewer app (`ichnos-app`), starting it if
//! needed. `ichnos stop` quits it.
//! `ichnos install` / `uninstall` wire an agent's hooks to `ichnos hook`, plus the
//! `skill/SKILL.md` skill that lets agents query the logs with SQL.
//!
//! Each session gets its own database, `~/.ichnos/sessions/<session-id>.db`.

mod adapter;
mod open;

use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;
use clap::{Parser, Subcommand};
use ichnos_core::error_log::{self, ERROR_LOG};
use ichnos_core::instance::View;
use ichnos_core::log::{Logged, Logger};
use ichnos_core::store::SessionId;
use serde_json::{Map, Value};

use crate::adapter::Adapter;
use crate::adapter::claude_code::{self, install};

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Data directory; sessions go to `<dir>/sessions/<session-id>.db` [default: ~/.ichnos].
    #[arg(long, global = true, env = "ICHNOS_DIR")]
    dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Read Ichnos events, one JSON object per line, from stdin and store them. Always exits 0.
    ///
    /// A new session also opens the viewer on it, like `ichnos open <session>`, unless the
    /// viewer is already running.
    Log {
        /// Don't open the viewer when a session starts.
        #[arg(long, env = "ICHNOS_NO_OPEN", value_parser = clap::builder::FalseyValueParser::new())]
        no_open: bool,
    },
    /// Read one hook payload of an agent from stdin, translate it and store it. Always exits 0.
    ///
    /// A new session also opens the viewer on it, like `ichnos log`.
    Hook {
        /// The agent that sent the payload.
        #[arg(value_enum, default_value_t)]
        adapter: Adapter,
        /// Don't open the viewer when a session starts.
        #[arg(long, env = "ICHNOS_NO_OPEN", value_parser = clap::builder::FalseyValueParser::new())]
        no_open: bool,
    },
    /// Log the parts of a recorded session an adapter can still find, such as Claude Code's
    /// model messages for sessions recorded before Ichnos logged them.
    Import {
        /// The agent that recorded the session.
        #[arg(value_enum)]
        adapter: Adapter,
        /// The session to complete.
        session: SessionId,
    },
    /// Show the viewer app, starting `ichnos-app` in the background if it isn't running.
    Open {
        /// Session to show; the window stays on it [default: follow the newest session].
        session: Option<SessionId>,
        /// View to show: tree, graph or table.
        #[arg(long)]
        view: Option<View>,
        /// Do nothing when the viewer is already running (how the hook runs it).
        #[arg(long)]
        unless_open: bool,
        /// Start the viewer without the one-line Haiku summaries.
        #[arg(long)]
        no_summaries: bool,
        /// Command line that writes the one-line summaries (prompt on stdin, reply on stdout)
        /// [default: `claude -p --model haiku …`].
        #[arg(long, env = "ICHNOS_SUMMARIZER")]
        summarizer: Option<String>,
    },
    /// Quit the viewer app.
    Stop,
    /// Add the ichnos hooks to an agent's settings, so new sessions are logged, and the skill
    /// that lets agents query the logs.
    Install {
        /// The agent to wire up.
        #[arg(value_enum, default_value_t)]
        adapter: Adapter,
        #[command(flatten)]
        target: SettingsTarget,
    },
    /// Remove the ichnos hooks and skill. Recorded sessions are kept.
    Uninstall {
        /// The agent to unwire.
        #[arg(value_enum, default_value_t)]
        adapter: Adapter,
        #[command(flatten)]
        target: SettingsTarget,
    },
}

/// Which Claude Code settings file `install` and `uninstall` change.
#[derive(Debug, clap::Args)]
struct SettingsTarget {
    /// Only this project: `.claude/settings.local.json` in the current directory
    /// [default: every session, `~/.claude/settings.json`].
    #[arg(long, conflicts_with = "settings")]
    project: bool,
    /// Change this settings file instead.
    #[arg(long, value_name = "FILE")]
    settings: Option<PathBuf>,
}

impl SettingsTarget {
    /// The settings file to change; the user file honors `CLAUDE_CONFIG_DIR` like Claude Code does.
    fn path(self) -> Option<PathBuf> {
        if let Some(path) = self.settings {
            return Some(path);
        }
        if self.project {
            return Some(PathBuf::from(".claude").join("settings.local.json"));
        }
        let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))?;
        Some(config_dir.join("settings.json"))
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // Exit code 2 from a PreToolUse hook would block the tool call, so a misconfigured
        // hook command (e.g. an unknown flag) still exits 0.
        Err(err) if std::env::args().any(|arg| arg == "hook" || arg == "log") => {
            eprintln!("{err}");
            return ExitCode::SUCCESS;
        }
        Err(err) => err.exit(),
    };
    // These don't touch the data directory.
    let command = match cli.command {
        Command::Install {
            adapter: Adapter::ClaudeCode,
            target,
        } => return change_settings(target, install::install, true),
        Command::Uninstall {
            adapter: Adapter::ClaudeCode,
            target,
        } => return change_settings(target, install::uninstall, false),
        command => command,
    };
    let Some(dir) = cli.dir.or_else(ichnos_core::default_data_dir) else {
        eprintln!("no data directory: set HOME, --dir or ICHNOS_DIR");
        return ExitCode::FAILURE;
    };
    let sessions_dir = dir.join(ichnos_core::SESSIONS_DIR);
    if let Err(err) = fs::create_dir_all(&sessions_dir) {
        // A hook still exits 0; there is nowhere to log the failure.
        eprintln!("could not create {}: {err}", sessions_dir.display());
        return if matches!(command, Command::Hook { .. } | Command::Log { .. }) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    match command {
        Command::Log { no_open } => {
            log_stdin(&dir, &sessions_dir, no_open);
            ExitCode::SUCCESS
        }
        Command::Hook { adapter, no_open } => {
            hook(&dir, &sessions_dir, adapter, no_open);
            ExitCode::SUCCESS
        }
        Command::Import {
            adapter: Adapter::ClaudeCode,
            session,
        } => match claude_code::import(&mut Logger::new(&sessions_dir), &session) {
            Ok(count) => {
                println!("Logged {count} model messages of session {session}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{}", error_chain(&err));
                ExitCode::FAILURE
            }
        },
        Command::Open {
            session,
            view,
            unless_open,
            no_summaries,
            summarizer,
        } => match open::run(
            &dir,
            open::OpenArgs {
                session: session.as_ref(),
                view,
                unless_open,
                no_summaries,
                summarizer: summarizer.as_deref(),
            },
        ) {
            Ok(open::Opened::Shown) => {
                println!("Showing it in the running viewer");
                ExitCode::SUCCESS
            }
            Ok(open::Opened::Started) => {
                println!("Started the viewer");
                ExitCode::SUCCESS
            }
            Ok(open::Opened::AlreadyOpen) => {
                println!("The viewer is already running");
                ExitCode::SUCCESS
            }
            Err(err) => {
                // Run from the hook, this lands in the error log, so it gets a timestamp.
                eprintln!("{} open failed: {err:?}", chrono::Utc::now().to_rfc3339());
                ExitCode::FAILURE
            }
        },
        Command::Stop => stop(&dir),
        Command::Install { .. } | Command::Uninstall { .. } => {
            unreachable!("handled above")
        }
    }
}

/// Store the Ichnos events on stdin, one per line. Never breaks the producer: failures go to the
/// error log in `dir`.
fn log_stdin(dir: &Path, sessions_dir: &Path, no_open: bool) {
    let mut logger = Logger::new(sessions_dir);
    for line in std::io::stdin().lock().lines() {
        let result = line
            .map_err(|err| format!("could not read stdin: {err}"))
            .and_then(|text| parse_object(&text))
            .and_then(|event| match event {
                None => Ok(Logged::Stored),
                Some(event) => logger
                    .log(&event, Utc::now())
                    .map_err(|err| format!("{err:?}")),
            });
        match result {
            Ok(outcome) => open_if_started(dir, &outcome, no_open),
            Err(err) => error_log::append(dir, &err),
        }
    }
}

/// Store the one hook payload on stdin through `adapter`. Never breaks the agent loop: failures
/// go to the error log in `dir`.
fn hook(dir: &Path, sessions_dir: &Path, adapter: Adapter, no_open: bool) {
    let mut raw = String::new();
    let result = std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|err| format!("could not read the hook payload from stdin: {err}"))
        .and_then(|_| parse_object(&raw))
        .and_then(|payload| {
            let payload = payload.unwrap_or_default();
            let mut logger = Logger::new(sessions_dir);
            match adapter {
                Adapter::ClaudeCode => claude_code::hook(&mut logger, &payload, Utc::now()),
            }
            .map_err(|err| format!("{err:?}"))
        });
    match result {
        Ok(outcome) => open_if_started(dir, &outcome, no_open),
        Err(err) => error_log::append(dir, &err),
    }
}

/// `line` parsed as a JSON object; `None` for a blank line.
fn parse_object(line: &str) -> Result<Option<Map<String, Value>>, String> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(line)
        .map(Some)
        .map_err(|err| format!("the input is not a JSON object: {err}"))
}

/// Open the viewer on a session that `logged` says just started, unless `no_open`.
fn open_if_started(dir: &Path, logged: &Logged, no_open: bool) {
    if let Logged::SessionStarted(session) = logged
        && !no_open
        && let Err(err) = open::spawn(dir, session, &dir.join(ERROR_LOG))
    {
        error_log::append(dir, &format!("{err:?}"));
    }
}

/// `err` and its causes on one line, separated by `: `.
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let _ = write!(text, ": {cause}");
        source = cause.source();
    }
    text
}

/// Quit the viewer app serving data directory `dir` and say what happened.
fn stop(dir: &Path) -> ExitCode {
    match open::stop(dir) {
        Ok(true) => {
            println!("Stopped the ichnos viewer");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("No ichnos viewer is running");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// Run `install` or `uninstall` on the chosen settings file and say what happened.
fn change_settings(
    target: SettingsTarget,
    change: fn(&Path) -> Result<install::Change, install::InstallError>,
    installing: bool,
) -> ExitCode {
    let Some(path) = target.path() else {
        eprintln!("no settings file: set HOME, CLAUDE_CONFIG_DIR or --settings");
        return ExitCode::FAILURE;
    };
    let shown = path.display();
    let skill = install::skill_path(&path);
    let skill = skill.display();
    match (change(&path), installing) {
        (Ok(install::Change::Updated), true) => {
            println!("Added the ichnos hooks to {shown} and the ichnos skill to {skill}.");
            println!(
                "New Claude Code sessions will be logged and open in the viewer; agents can query the logs with the skill."
            );
        }
        (Ok(install::Change::Unchanged), true) => {
            println!("The ichnos hooks are already in {shown} and the skill in {skill}.");
        }
        (Ok(install::Change::Updated), false) => {
            println!("Removed the ichnos hooks from {shown} and the skill from {skill}.");
            println!("New Claude Code sessions won't be logged; recorded ones stay in ~/.ichnos.");
        }
        (Ok(install::Change::Unchanged), false) => {
            println!("There are no ichnos hooks in {shown} and no skill in {skill}.");
        }
        (Err(err), _) => {
            eprintln!("{}", error_chain(&err));
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
