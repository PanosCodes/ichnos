//! A language model as a shell command: the prompt goes to stdin, the reply comes back on stdout.
//!
//! Ichnos doesn't name a model provider. Whoever starts the summarizer chooses the command
//! (`claude -p`, `llm -m …`, a script calling an API), and the system prompt reaches it in the
//! `ICHNOS_SYSTEM_PROMPT` environment variable, for commands that take one
//! (`--system-prompt "$ICHNOS_SYSTEM_PROMPT"`).

use std::ffi::OsString;
use std::io::{self, Read, Write as _};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

/// How long one command run may take before it is killed.
pub const LLM_TIMEOUT: Duration = Duration::from_secs(120);

/// The environment variable that carries the system prompt to the command.
pub const SYSTEM_PROMPT_VAR: &str = "ICHNOS_SYSTEM_PROMPT";

/// Characters of the command's stderr kept in an [`LlmError::Exit`].
const STDERR_CHARS: usize = 500;

/// How often a running command is checked for having exited.
const EXIT_POLL: Duration = Duration::from_millis(50);

/// Why the command gave no reply.
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("could not run `{command}`")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },

    #[error("`{command}` did not answer within {LLM_TIMEOUT:?}")]
    Timeout { command: String },

    #[error("`{command}` exited with {status}: {stderr}")]
    Exit {
        command: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
}

/// A shell command line that answers a prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Llm {
    command_line: String,
    /// Environment variables set for the command on top of the inherited ones.
    env: Vec<(String, OsString)>,
}

impl Llm {
    /// The command line `command_line`, run with `sh -c` in the temporary directory, so an agent
    /// CLI doesn't pick up the current project.
    #[must_use]
    pub fn new(command_line: impl Into<String>) -> Self {
        Self {
            command_line: command_line.into(),
            env: Vec::new(),
        }
    }

    /// Also set the environment variable `name` to `value` for the command, such as a `PATH`
    /// that finds the user's tools when the app was started outside a terminal.
    #[must_use]
    pub fn env(mut self, name: impl Into<String>, value: impl Into<OsString>) -> Self {
        self.env.push((name.into(), value.into()));
        self
    }

    /// The command's stdout for `prompt`, with `system` in [`SYSTEM_PROMPT_VAR`].
    ///
    /// # Errors
    ///
    /// Returns an [`LlmError`] when the command can't start, fails, or takes longer than
    /// [`LLM_TIMEOUT`].
    pub fn complete(&self, system: &str, prompt: &str) -> Result<String, LlmError> {
        let spawn_error = |source| LlmError::Spawn {
            command: self.command_line.clone(),
            source,
        };
        let mut child = Command::new("sh")
            .args(["-c", &self.command_line])
            .envs(self.env.iter().map(|(name, value)| (name, value)))
            .env(SYSTEM_PROMPT_VAR, system)
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(spawn_error)?;
        // Each pipe gets a thread, so a long prompt or reply can't fill one while the other waits.
        let mut stdin = child.stdin.take();
        let prompt = prompt.to_owned();
        let writer = thread::spawn(move || {
            stdin
                .as_mut()
                .map_or(Ok(()), |stdin| stdin.write_all(prompt.as_bytes()))
        });
        let stdout = read_all(child.stdout.take());
        let stderr = read_all(child.stderr.take());
        let deadline = Instant::now() + LLM_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(spawn_error)? {
                break status;
            }
            if Instant::now() >= deadline {
                // Already gone or not killable: either way there is no reply to wait for.
                let _ = child.kill();
                let _ = child.wait();
                return Err(LlmError::Timeout {
                    command: self.command_line.clone(),
                });
            }
            thread::sleep(EXIT_POLL);
        };
        if !status.success() {
            return Err(LlmError::Exit {
                command: self.command_line.clone(),
                status,
                stderr: String::from_utf8_lossy(&stderr.join().unwrap_or_default())
                    .trim()
                    .chars()
                    .take(STDERR_CHARS)
                    .collect(),
            });
        }
        writer.join().unwrap_or(Ok(())).map_err(spawn_error)?;
        Ok(String::from_utf8_lossy(&stdout.join().unwrap_or_default()).into_owned())
    }
}

/// Everything `pipe` yields until it closes, read on a thread of its own.
fn read_all(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            // A read error ends the output early; the exit status still says how the run went.
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_goes_to_stdin_and_the_system_prompt_to_the_environment() {
        let llm = Llm::new(r#"printf '%s|' "$ICHNOS_SYSTEM_PROMPT"; cat"#);
        assert_eq!(llm.complete("be brief", "hi").unwrap(), "be brief|hi");
    }

    #[test]
    fn extra_environment_variables_reach_the_command() {
        let llm = Llm::new(r#"cat >/dev/null; printf %s "$GREETING""#).env("GREETING", "hello");
        assert_eq!(llm.complete("", "").unwrap(), "hello");
    }

    #[test]
    fn a_failing_command_reports_its_stderr() {
        let llm = Llm::new("echo nope >&2; exit 3");
        let err = llm.complete("", "").unwrap_err();
        assert!(matches!(err, LlmError::Exit { ref stderr, .. } if stderr == "nope"));
    }
}
