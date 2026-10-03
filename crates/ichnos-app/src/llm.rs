//! A language model as a shell command: the prompt goes to stdin, the reply comes back on stdout.
//!
//! Ichnos doesn't name a model provider. Whoever starts the summarizer chooses the command
//! (`claude -p`, `llm -m …`, a script calling an API), and the system prompt reaches it in the
//! `ICHNOS_SYSTEM_PROMPT` environment variable, for commands that take one
//! (`--system-prompt "$ICHNOS_SYSTEM_PROMPT"`).

use std::io;
use std::process::Stdio;
use std::time::Duration;

use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// How long one command run may take before it is killed.
pub const LLM_TIMEOUT: Duration = Duration::from_secs(120);

/// The environment variable that carries the system prompt to the command.
pub const SYSTEM_PROMPT_VAR: &str = "ICHNOS_SYSTEM_PROMPT";

/// Characters of the command's stderr kept in an [`LlmError::Exit`].
const STDERR_CHARS: usize = 500;

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
}

impl Llm {
    /// The command line `command_line`, run with `sh -c` in the temporary directory, so an agent
    /// CLI doesn't pick up the current project.
    #[must_use]
    pub fn new(command_line: impl Into<String>) -> Self {
        Self {
            command_line: command_line.into(),
        }
    }

    /// The command's stdout for `prompt`, with `system` in [`SYSTEM_PROMPT_VAR`].
    ///
    /// # Errors
    ///
    /// Returns an [`LlmError`] when the command can't start, fails, or takes longer than
    /// [`LLM_TIMEOUT`].
    pub async fn complete(&self, system: &str, prompt: &str) -> Result<String, LlmError> {
        let spawn_error = |source| LlmError::Spawn {
            command: self.command_line.clone(),
            source,
        };
        let mut child = Command::new("sh")
            .args(["-c", &self.command_line])
            .env(SYSTEM_PROMPT_VAR, system)
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(spawn_error)?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(spawn_error)?;
        }
        let output = tokio::time::timeout(LLM_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| LlmError::Timeout {
                command: self.command_line.clone(),
            })?
            .map_err(spawn_error)?;
        if !output.status.success() {
            return Err(LlmError::Exit {
                command: self.command_line.clone(),
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr)
                    .trim()
                    .chars()
                    .take(STDERR_CHARS)
                    .collect(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_prompt_goes_to_stdin_and_the_system_prompt_to_the_environment() {
        let llm = Llm::new(r#"printf '%s|' "$ICHNOS_SYSTEM_PROMPT"; cat"#);
        assert_eq!(llm.complete("be brief", "hi").await.unwrap(), "be brief|hi");
    }

    #[tokio::test]
    async fn a_failing_command_reports_its_stderr() {
        let llm = Llm::new("echo nope >&2; exit 3");
        let err = llm.complete("", "").await.unwrap_err();
        assert!(matches!(err, LlmError::Exit { ref stderr, .. } if stderr == "nope"));
    }
}
