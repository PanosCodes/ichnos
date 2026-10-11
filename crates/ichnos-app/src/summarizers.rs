//! The summarizers the user can pick in Settings: their command lines, batch sizes, saved keys and
//! whether this Mac can run them. The only place that names a model provider.

use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::llm::Llm;
use crate::summarizer::SummaryModel;

/// Haiku through the Claude Code CLI, with no tools, no MCP servers, hooks disabled and only user
/// settings, so the summarizing session never shows up in the logs itself. The system prompt
/// replaces Claude Code's own.
const CLAUDE_CODE: &str = concat!(
    "MAX_THINKING_TOKENS=0 claude -p --model haiku --tools '' --strict-mcp-config ",
    "--setting-sources user --settings '{\"disableAllHooks\":true,\"alwaysThinkingEnabled\":false}' ",
    "--no-session-persistence --disable-slash-commands ",
    "--system-prompt \"$ICHNOS_SYSTEM_PROMPT\" --output-format text",
);

/// The on-device Apple Foundation Model through the `fm` CLI of macOS 26. Greedy sampling keeps
/// the small model closer to the JSON it is asked for.
const APPLE_FOUNDATION_MODELS: &str =
    "fm respond --no-stream --greedy --instructions \"$ICHNOS_SYSTEM_PROMPT\"";

/// Entries per request for a large hosted model.
const LARGE_BATCH: usize = 12;
/// Entries per request for the on-device model, whose context holds about 4,000 tokens.
const SMALL_BATCH: usize = 3;

/// Marks the `PATH` in the login shell's output, which may also hold whatever its startup files
/// print.
const PATH_MARK: &str = "__ICHNOS_PATH__";
/// How long the login shell may take to print its `PATH`.
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(3);

/// A summarizer the user can pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Haiku through `claude -p`.
    ClaudeCode,
    /// The on-device model through `fm respond`.
    AppleFoundationModels,
    /// The command line from `--summarizer` or `ICHNOS_SUMMARIZER`; never saved.
    Command(String),
    /// No summaries.
    Off,
}

/// Whether a choice can run on this Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Ready,
    /// Why not, in words for the settings window.
    Missing(&'static str),
}

/// Checks whether one choice can run; it may start a process, so call it off the UI thread.
pub type Probe = fn(&Choice) -> Availability;

impl Choice {
    /// The name shown in the dropdown.
    pub fn label(&self) -> String {
        match self {
            Self::ClaudeCode => "Claude Code (Haiku)".to_owned(),
            Self::AppleFoundationModels => "Apple Foundation Models".to_owned(),
            Self::Command(_) => "Custom command".to_owned(),
            Self::Off => "Off".to_owned(),
        }
    }

    /// The model to ask, or `None` for [`Choice::Off`].
    pub fn model(&self) -> Option<SummaryModel> {
        let (command_line, batch_size) = match self {
            Self::ClaudeCode => (CLAUDE_CODE, LARGE_BATCH),
            Self::AppleFoundationModels => (APPLE_FOUNDATION_MODELS, SMALL_BATCH),
            Self::Command(command_line) => (command_line.as_str(), LARGE_BATCH),
            Self::Off => return None,
        };
        let llm = Llm::new(command_line);
        Some(SummaryModel {
            llm: match login_path() {
                Some(path) => llm.env("PATH", path),
                None => llm,
            },
            batch_size,
        })
    }

    /// The key saved in the settings file, or `None` for a [`Choice::Command`], which comes from
    /// the command line each launch.
    pub fn key(&self) -> Option<&'static str> {
        match self {
            Self::ClaudeCode => Some("claude-code"),
            Self::AppleFoundationModels => Some("apple-foundation-models"),
            Self::Command(_) => None,
            Self::Off => Some("off"),
        }
    }

    /// The choice saved under `key`.
    pub fn from_key(key: &str) -> Option<Self> {
        [Self::ClaudeCode, Self::AppleFoundationModels, Self::Off]
            .into_iter()
            .find(|choice| choice.key() == Some(key))
    }
}

/// The choices to list: a `--summarizer` command first when there is one, then the built-in ones.
pub fn choices(command_line: Option<&str>) -> Vec<Choice> {
    command_line
        .map(|command_line| Choice::Command(command_line.to_owned()))
        .into_iter()
        .chain([
            Choice::ClaudeCode,
            Choice::AppleFoundationModels,
            Choice::Off,
        ])
        .collect()
}

/// The summarizer to start with: a `--summarizer` command, else the saved one, else Claude Code.
pub fn initial(command_line: Option<&str>, saved: Option<&str>) -> Choice {
    command_line
        .map(|command_line| Choice::Command(command_line.to_owned()))
        .or_else(|| saved.and_then(Choice::from_key))
        .unwrap_or(Choice::ClaudeCode)
}

/// Whether `choice` can run here: `claude` must be on the `PATH` the summarizer's shell sees, and
/// `fm` must report the system model available (it exists from macOS 26 and needs Apple
/// Intelligence on).
pub fn probe(choice: &Choice) -> Availability {
    let succeeds = |program: &str, args: &[&str]| {
        let mut command = Command::new(program);
        if let Some(path) = login_path() {
            command.env("PATH", path);
        }
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    match choice {
        Choice::ClaudeCode if !succeeds("sh", &["-c", "command -v claude"]) => {
            Availability::Missing("Needs Claude Code: no claude command was found.")
        }
        Choice::AppleFoundationModels if !succeeds("fm", &["available", "--model", "system"]) => {
            Availability::Missing("Needs macOS 26 or later with Apple Intelligence turned on.")
        }
        _ => Availability::Ready,
    }
}

/// The `PATH` of the user's interactive login shell, read once, or `None` when it can't be read.
///
/// An app started from the Finder or the Dock gets only the system `PATH`, while tools such as
/// `claude` are often added to it in `~/.zshrc`, which only an interactive shell reads. Takes
/// about half a second the first time, so call it off the UI thread first.
pub fn login_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(read_login_path).as_deref()
}

fn read_login_path() -> Option<String> {
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/zsh".into());
    let child = Command::new(shell)
        .args([
            "-i",
            "-l",
            "-c",
            &format!(r#"printf '{PATH_MARK}%s{PATH_MARK}' "$PATH""#),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // A startup file that waits for input must not hold the summarizer up.
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || sender.send(child.wait_with_output()));
    let output = receiver.recv_timeout(LOGIN_SHELL_TIMEOUT).ok()?.ok()?;
    marked_path(&String::from_utf8_lossy(&output.stdout)).map(str::to_owned)
}

/// The text between the first two [`PATH_MARK`]s of `output`, unless it is empty.
fn marked_path(output: &str) -> Option<&str> {
    let (_, rest) = output.split_once(PATH_MARK)?;
    let (path, _) = rest.split_once(PATH_MARK)?;
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_keys_round_trip_and_commands_are_not_saved() {
        for choice in choices(None) {
            assert_eq!(Choice::from_key(choice.key().unwrap()), Some(choice));
        }
        assert_eq!(Choice::Command("cat".into()).key(), None);
        assert_eq!(Choice::from_key("gpt"), None);
    }

    #[test]
    fn a_command_line_wins_over_the_saved_choice() {
        assert_eq!(
            initial(Some("cat"), Some("off")),
            Choice::Command("cat".into())
        );
        assert_eq!(initial(None, Some("off")), Choice::Off);
        assert_eq!(initial(None, Some("unknown")), Choice::ClaudeCode);
        assert_eq!(initial(None, None), Choice::ClaudeCode);
    }

    #[test]
    fn a_command_line_is_listed_first() {
        assert_eq!(
            choices(Some("cat")),
            [
                Choice::Command("cat".into()),
                Choice::ClaudeCode,
                Choice::AppleFoundationModels,
                Choice::Off
            ]
        );
    }

    #[test]
    fn the_on_device_model_gets_a_small_batch_and_off_gets_none() {
        let small = Choice::AppleFoundationModels.model().unwrap().batch_size;
        assert!(small < Choice::ClaudeCode.model().unwrap().batch_size);
        assert_eq!(Choice::Off.model(), None);
    }

    #[test]
    fn the_path_is_read_from_between_the_marks() {
        let output = format!("Welcome!\n{PATH_MARK}/a/bin:/usr/bin{PATH_MARK}\nbye");
        assert_eq!(marked_path(&output), Some("/a/bin:/usr/bin"));
        assert_eq!(marked_path(&format!("{PATH_MARK}{PATH_MARK}")), None);
        assert_eq!(marked_path("no marks"), None);
    }

    #[test]
    fn commands_and_off_need_no_check() {
        assert_eq!(probe(&Choice::Off), Availability::Ready);
        assert_eq!(probe(&Choice::Command("x".into())), Availability::Ready);
    }
}
