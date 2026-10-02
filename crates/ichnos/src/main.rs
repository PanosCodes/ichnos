//! Logs every Claude Code agent event to an SQLite database.
//!
//! `ichnos hook` is the Claude Code hook (wired from the block in `hooks.json`).
//!
//! Each Claude Code session gets its own database, `~/.ichnos/sessions/<session-id>.db`.

mod hook;

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use ichnos_core::error_log;

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
    /// Read one hook event from stdin and store it. Always exits 0.
    Hook,
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // Exit code 2 from a PreToolUse hook would block the tool call, so a misconfigured
        // hook command (e.g. an unknown flag) still exits 0.
        Err(err) if std::env::args().any(|arg| arg == "hook") => {
            eprintln!("{err}");
            return ExitCode::SUCCESS;
        }
        Err(err) => err.exit(),
    };
    let command = cli.command;
    let Some(dir) = cli.dir.or_else(ichnos_core::default_data_dir) else {
        eprintln!("no data directory: set HOME, --dir or ICHNOS_DIR");
        return ExitCode::FAILURE;
    };
    let sessions_dir = dir.join(ichnos_core::SESSIONS_DIR);
    if let Err(err) = fs::create_dir_all(&sessions_dir) {
        // A hook still exits 0; there is nowhere to log the failure.
        eprintln!("could not create {}: {err}", sessions_dir.display());
        return if matches!(command, Command::Hook) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    match command {
        Command::Hook => {
            // Never break the agent loop: failures go to the error log and the hook exits 0.
            if let Err(err) = hook::run(&sessions_dir, std::io::stdin().lock()) {
                error_log::append(&dir, &format!("{err:?}"));
            }
            ExitCode::SUCCESS
        }
    }
}
