//! The model's own account of a turn, in the shape `docs/ichnos-debug.schema.json` describes.
//!
//! The account is split into [`Section`]s, and a [`Profile`] picks the ones worth paying for.
//! A model is asked for it in the [`compact`] form, which Ichnos expands to the full form it
//! stores. Nothing here knows which agent is asked.

pub mod compact;

use serde_json::Value;

/// The event name of an account given after a turn, with the account in `extra.debug` and the
/// turn's id in `extra.turn_id`.
pub const DEBUG_EVENT: &str = "Debug";

/// A part of the account that answers one question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    /// Did it understand what was asked?
    Task,
    /// What did it choose, and on what basis?
    Decisions,
    /// What did it rely on, and what was missing?
    Context,
    /// Where did the environment get in its way?
    Tools,
    /// Did the skills work?
    Skills,
    /// Which instructions disagreed?
    Rules,
    /// Is it finished, and how sure is it?
    Outcome,
}

impl Section {
    /// Every section, in the order the block lists them.
    pub const ALL: [Self; 7] = [
        Self::Task,
        Self::Decisions,
        Self::Context,
        Self::Tools,
        Self::Skills,
        Self::Rules,
        Self::Outcome,
    ];

    /// The section's key in the block.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Decisions => "decisions",
            Self::Context => "context",
            Self::Tools => "tools",
            Self::Skills => "skills",
            Self::Rules => "rules",
            Self::Outcome => "outcome",
        }
    }

    /// The question the section answers.
    #[must_use]
    pub fn question(self) -> &'static str {
        match self {
            Self::Task => "Did you understand what was asked?",
            Self::Decisions => "What did you choose, and on what basis?",
            Self::Context => "What did you rely on, and what was missing?",
            Self::Tools => "Where did the environment get in your way?",
            Self::Skills => "Did the skills work?",
            Self::Rules => "Which instructions disagreed?",
            Self::Outcome => "Is it finished, and how sure are you?",
        }
    }
}

/// A set of sections chosen for one purpose, so a session pays only for what it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Profile {
    /// Tuning skills: `skills`, `rules`, `outcome`.
    Skills,
    /// Investigating corrections and misunderstandings: `task`, `decisions`, `outcome`.
    Task,
    /// Tuning instruction files, permissions and tools: `context`, `tools`.
    Env,
    /// Every section. Long, and changes the model's behavior the most.
    Full,
}

impl Profile {
    /// Every profile.
    pub const ALL: [Self; 4] = [Self::Skills, Self::Task, Self::Env, Self::Full];

    /// The profile's name, as a setting would spell it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Skills => "skills",
            Self::Task => "task",
            Self::Env => "env",
            Self::Full => "full",
        }
    }

    /// The sections the profile asks for, in block order.
    #[must_use]
    pub fn sections(self) -> &'static [Section] {
        match self {
            Self::Skills => &[Section::Skills, Section::Rules, Section::Outcome],
            Self::Task => &[Section::Task, Section::Decisions, Section::Outcome],
            Self::Env => &[Section::Context, Section::Tools],
            Self::Full => &Section::ALL,
        }
    }
}

/// A one-line summary of a full-form account: its outcome and how many entries it reported,
/// such as `done · 2 reported`, or `invalid` for one that wasn't readable.
#[must_use]
pub fn summary(debug: &Value) -> String {
    if debug.get("invalid") == Some(&Value::Bool(true)) {
        return "invalid".to_owned();
    }
    let state = debug
        .pointer("/outcome/state")
        .and_then(Value::as_str)
        .unwrap_or("no outcome");
    let reported: usize = debug
        .as_object()
        .into_iter()
        .flat_map(|sections| sections.values())
        .filter_map(Value::as_object)
        .flat_map(|fields| fields.values())
        .filter_map(Value::as_array)
        .map(Vec::len)
        .sum();
    if reported == 0 {
        state.to_owned()
    } else {
        format!("{state} · {reported} reported")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SCHEMA: &str = include_str!("../../../docs/ichnos-debug.schema.json");

    fn validator() -> jsonschema::Validator {
        jsonschema::validator_for(&serde_json::from_str(SCHEMA).unwrap()).unwrap()
    }

    #[test]
    fn the_schema_rejects_keys_it_does_not_define() {
        let validator = validator();
        assert!(!validator.is_valid(&json!({ "v": 1, "confidence": 0.8 })));
        assert!(!validator.is_valid(&json!({
            "v": 1,
            "outcome": { "state": "done", "remaining": [], "unverified_claims": [], "risks": [], "mood": "good" }
        })));
    }

    #[test]
    fn a_summary_gives_the_outcome_and_how_much_was_reported() {
        let debug = json!({
            "v": 1,
            "skills": { "considered": [{ "skill": "x" }], "skipped_steps": [{ "step": "y" }] },
            "outcome": { "state": "partial", "remaining": [] }
        });
        assert_eq!(summary(&debug), "partial · 2 reported");
        assert_eq!(
            summary(&json!({ "v": 1, "outcome": { "state": "done" } })),
            "done"
        );
        assert_eq!(summary(&json!({ "invalid": true })), "invalid");
    }
}
