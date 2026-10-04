//! Which text of a session gets indexed: prompts, errors of failed calls and summaries.

use std::fmt;
use std::str::FromStr;

use ichnos_core::event::{Event, Status};
use ichnos_core::store::Row;
use serde_json::Value;

/// The longest text indexed for one event, in characters; the rest is cut.
pub(crate) const DOC_CHARS: usize = 500;

/// What a piece of indexed text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocKind {
    /// A prompt the user typed.
    Prompt,
    /// The error of a failed tool call.
    Error,
    /// The one-line summary of an entry.
    Summary,
}

impl DocKind {
    /// The kind as stored and shown: `prompt`, `error` or `summary`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Error => "error",
            Self::Summary => "summary",
        }
    }
}

impl fmt::Display for DocKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DocKind {
    type Err = String;

    fn from_str(kind: &str) -> Result<Self, Self::Err> {
        match kind {
            "prompt" => Ok(Self::Prompt),
            "error" => Ok(Self::Error),
            "summary" => Ok(Self::Summary),
            other => Err(format!("unknown doc kind {other:?}")),
        }
    }
}

/// One piece of text to index, tied to the event row it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Doc {
    /// The session's `events.id` the text belongs to.
    pub event_id: i64,
    pub kind: DocKind,
    pub text: String,
}

/// The docs in new event and summary rows: prompts, errors of failed calls and summaries.
pub(crate) fn docs_from(events: &[Row], summaries: &[Row]) -> Vec<Doc> {
    let from_events = events.iter().filter_map(|row| {
        let event = Event::from_row(row).ok()?;
        let (kind, text) = if event.name() == "UserPromptSubmit" {
            (DocKind::Prompt, as_text(event.prompt())?)
        } else if event.status() == Some(Status::Error) {
            let error = as_text(event.error())?;
            let text = match event.tool_name() {
                Some(tool) => format!("{tool}: {error}"),
                None => error,
            };
            (DocKind::Error, text)
        } else {
            return None;
        };
        doc(row.id, kind, &text)
    });
    let from_summaries = summaries.iter().filter_map(|row| {
        let summary: Value = serde_json::from_str(&row.json).ok()?;
        let event_id = summary.get("event_id")?.as_i64()?;
        doc(
            event_id,
            DocKind::Summary,
            &as_text(summary.get("summary")?)?,
        )
    });
    from_events.chain(from_summaries).collect()
}

/// `text` on one line and cut to [`DOC_CHARS`]; `None` when it is blank.
fn doc(event_id: i64, kind: DocKind, text: &str) -> Option<Doc> {
    let text: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(DOC_CHARS)
        .collect();
    (!text.is_empty()).then_some(Doc {
        event_id,
        kind,
        text,
    })
}

/// A string value as is, any other non-null value as JSON.
fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn row(id: i64, json: &Value) -> Row {
        Row {
            id,
            json: json.to_string(),
        }
    }

    fn event(name: &str, fields: &Value) -> Value {
        let mut event = json!({"event": name, "epoch_ms": 1, "session_id": "s"});
        event
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        event
    }

    #[test]
    fn prompts_and_errors_of_failed_calls_are_indexed() {
        let events = [
            row(
                1,
                &event(
                    "UserPromptSubmit",
                    &json!({"prompt": "fix the\n  login bug"}),
                ),
            ),
            row(
                2,
                &event(
                    "PreToolUse",
                    &json!({"tool_use_id": "t1", "tool_name": "Bash", "status": "started"}),
                ),
            ),
            row(
                3,
                &event(
                    "PostToolUseFailure",
                    &json!({"tool_use_id": "t1", "tool_name": "Bash", "status": "error", "error": "exit 1"}),
                ),
            ),
            row(
                4,
                &event(
                    "PostToolUse",
                    &json!({"tool_use_id": "t2", "tool_name": "Read", "status": "ok"}),
                ),
            ),
        ];
        assert_eq!(
            docs_from(&events, &[]),
            [
                Doc {
                    event_id: 1,
                    kind: DocKind::Prompt,
                    text: "fix the login bug".to_owned()
                },
                Doc {
                    event_id: 3,
                    kind: DocKind::Error,
                    text: "Bash: exit 1".to_owned()
                },
            ]
        );
    }

    #[test]
    fn summaries_are_indexed_under_the_event_they_describe() {
        let summaries = [row(7, &json!({"event_id": 2, "summary": "Ran the tests."}))];
        assert_eq!(
            docs_from(&[], &summaries),
            [Doc {
                event_id: 2,
                kind: DocKind::Summary,
                text: "Ran the tests.".to_owned()
            }]
        );
    }

    #[test]
    fn long_text_is_cut_and_blank_text_is_skipped() {
        let long = "word ".repeat(DOC_CHARS);
        let events = [
            row(1, &event("UserPromptSubmit", &json!({"prompt": long}))),
            row(2, &event("UserPromptSubmit", &json!({"prompt": "   "}))),
        ];
        let docs = docs_from(&events, &[]);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].text.chars().count(), DOC_CHARS);
    }

    #[test]
    fn kinds_read_back_from_their_text() {
        for kind in [DocKind::Prompt, DocKind::Error, DocKind::Summary] {
            assert_eq!(kind.as_str().parse::<DocKind>(), Ok(kind));
        }
    }
}
