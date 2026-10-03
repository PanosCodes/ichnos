//! Adapters: translate a coding agent's own payloads into Ichnos events.
//!
//! Core ([`ichnos_core::log`]) knows only the Ichnos event format. An adapter exists for an agent
//! whose hooks can only run a command with the agent's own payload, so something has to reshape
//! it. Anything that can write Ichnos events itself pipes them into `ichnos log` instead and
//! needs no adapter.
//!
//! An adapter is a module with `hook` (one payload in, events logged), `install` / `uninstall`
//! (wire the agent's hooks to `ichnos hook <adapter>`) and, optionally, `import`. Adding one is a
//! new module, a new [`Adapter`] variant and its match arms in `main.rs`.

pub mod claude_code;

/// The agents Ichnos has an adapter for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Adapter {
    /// Claude Code, through its command hooks and transcripts.
    #[default]
    ClaudeCode,
}
