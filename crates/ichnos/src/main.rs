//! Logs every Claude Code agent event to an SQLite database.
//!
//! `ichnos hook` is the Claude Code hook (wired from the block in `hooks.json`).
//! `ichnos install` / `uninstall` add or remove the `hooks.json` block in Claude Code's settings
//! and the `skill/SKILL.md` skill next to it, which lets agents query the logs with SQL.
//!
//! Each Claude Code session gets its own database, `~/.ichnos/sessions/<session-id>.db`.

mod hook;
mod install;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
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
    /// Add the ichnos hooks to Claude Code's settings, so new sessions are logged, and the skill
    /// that lets agents query the logs.
    Install {
        #[command(flatten)]
        target: SettingsTarget,
    },
    /// Remove the ichnos hooks and skill. Recorded sessions are kept.
    Uninstall {
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
        Err(err) if std::env::args().any(|arg| arg == "hook") => {
            eprintln!("{err}");
            return ExitCode::SUCCESS;
        }
        Err(err) => err.exit(),
    };
    // These don't touch the data directory.
    let command = match cli.command {
        Command::Install { target } => return change_settings(target, install::install, true),
        Command::Uninstall { target } => return change_settings(target, install::uninstall, false),
        command @ Command::Hook => command,
    };
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
        Command::Install { .. } | Command::Uninstall { .. } => {
            unreachable!("handled above")
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
                "New Claude Code sessions will be logged; agents can query the logs with the skill."
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
