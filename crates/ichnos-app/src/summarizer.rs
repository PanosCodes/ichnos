//! Plain-language summaries of agent log entries, written by a language model.
//!
//! One background task summarizes the sessions someone is watching. It pairs Pre/Post tool events
//! into one entry (like the viewer does), skips calls that are still running and model messages,
//! sends the rest to the [`Llm`] command in batches (newest first, so the live end fills in first)
//! and stores the sentences in the database, where the viewer's feed picks them up.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Map, Value};
use thiserror::Error;

use crate::llm::{Llm, LlmError};
use ichnos_core::event::ASSISTANT_MESSAGE;
use ichnos_core::store::{Row, SessionId, Store, StoreError};

/// Entries sent to the model in one request.
const BATCH_SIZE: usize = 12;
/// Failed attempts after which an entry is left without a summary.
const MAX_ATTEMPTS: u32 = 2;
/// Characters of each field sent to the model; the rest is cut and its length noted.
const FIELD_CHARS: usize = 800;
/// Pause between rounds when there was nothing to summarize.
const IDLE_PAUSE: Duration = Duration::from_secs(1);

/// The instructions given to the model as its system prompt.
const SYSTEM_PROMPT: &str = "You explain log entries from an AI coding agent to a human who \
wants to follow what the agent did. English is not their first language.

For each entry write ONE short sentence (at most 25 words) in plain, simple English: what the \
agent did and what happened. Name the concrete thing (file, search term, command, test) and the \
outcome (found 3 matches, test failed, file created). Explain jargon instead of repeating it. \
Do not start with \"The agent\". Subagents are helper agents started by the main agent.

Reply with JSON only, no code fences: an object mapping each entry's \"key\" to its sentence.";

/// Why a batch could not be summarized.
#[derive(Debug, Error)]
pub enum SummarizeError {
    #[error("could not read or write summaries")]
    Store(#[from] StoreError),

    #[error("the model gave no reply")]
    Llm(#[from] LlmError),

    #[error("the model's reply is not the expected JSON")]
    Parse(#[from] serde_json::Error),

    #[error("no JSON object in the model's reply: {0}")]
    NoJson(String),

    #[error("the database task panicked")]
    Join(#[from] tokio::task::JoinError),
}

/// Handle to the background summarizer; clones share one worker.
#[derive(Debug, Clone)]
pub struct Summarizer {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// The sessions folder holding one `<session-id>.db` per session.
    dir: PathBuf,
    error_log_dir: PathBuf,
    llm: Llm,
    /// Session -> number of open streams watching it.
    watchers: Mutex<HashMap<SessionId, usize>>,
}

/// Keeps a session summarized while alive; dropping it stops watching.
#[derive(Debug)]
pub struct Watch {
    summarizer: Summarizer,
    session: SessionId,
}

impl Summarizer {
    /// Start the worker on the current tokio runtime, asking `llm` for summaries and storing them
    /// in each session's database inside the sessions folder `dir`. Failures go to
    /// `<error_log_dir>/hook-errors.log`.
    #[must_use]
    pub fn start(dir: PathBuf, error_log_dir: PathBuf, llm: Llm) -> Self {
        let summarizer = Self {
            inner: Arc::new(Inner {
                dir,
                error_log_dir,
                llm,
                watchers: Mutex::new(HashMap::new()),
            }),
        };
        tokio::spawn(summarizer.clone().run());
        summarizer
    }

    /// Summarize `session` until the returned guard is dropped.
    #[must_use]
    pub fn watch(&self, session: SessionId) -> Watch {
        *self.watchers().entry(session.clone()).or_insert(0) += 1;
        Watch {
            summarizer: self.clone(),
            session,
        }
    }

    fn watchers(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, usize>> {
        // The map stays consistent even if a holder panicked, so a poisoned lock is still usable.
        self.inner
            .watchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    async fn run(self) {
        let mut attempts: HashMap<i64, u32> = HashMap::new();
        loop {
            let sessions: Vec<SessionId> = self.watchers().keys().cloned().collect();
            let mut worked = false;
            for session in sessions {
                let db = session.db_path(&self.inner.dir);
                match summarize_next_batch(&self.inner.llm, &db, &mut attempts).await {
                    Ok(did_work) => worked |= did_work,
                    // Keep the worker alive; the viewer just lacks summaries.
                    Err(err) => {
                        ichnos_core::error_log::append(
                            &self.inner.error_log_dir,
                            &format!("summarizer {session}: {err:?}"),
                        );
                    }
                }
            }
            if !worked {
                tokio::time::sleep(IDLE_PAUSE).await;
            }
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let mut watchers = self.summarizer.watchers();
        if let Some(count) = watchers.get_mut(&self.session) {
            *count -= 1;
            if *count == 0 {
                watchers.remove(&self.session);
            }
        }
    }
}

/// One finished entry (a whole tool call, or a lifecycle event) ready to send to the model.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    /// Row id of the event that starts the entry; summaries are stored under it.
    event_id: i64,
    /// What the model sees; its `"key"` is `event_id` as text.
    fields: Value,
}

/// Summarize the newest batch of unsummarized entries; returns whether there was one.
async fn summarize_next_batch(
    llm: &Llm,
    db: &Path,
    attempts: &mut HashMap<i64, u32>,
) -> Result<bool, SummarizeError> {
    let db_path = db.to_owned();
    let (entries, done) = tokio::task::spawn_blocking(move || -> Result<_, SummarizeError> {
        let store = Store::open(&db_path)?;
        Ok((
            read_entries(&store.events_after(0)?),
            store.summarized_event_ids()?,
        ))
    })
    .await??;

    let todo: Vec<Entry> = entries
        .into_iter()
        .filter(|entry| {
            !done.contains(&entry.event_id)
                && attempts.get(&entry.event_id).copied().unwrap_or(0) < MAX_ATTEMPTS
        })
        .collect();
    if todo.is_empty() {
        return Ok(false);
    }
    let batch = &todo[todo.len().saturating_sub(BATCH_SIZE)..];
    let mut fail = |event_id: i64| *attempts.entry(event_id).or_insert(0) += 1;

    let fields: Vec<&Value> = batch.iter().map(|entry| &entry.fields).collect();
    let replies = match ask(llm, &fields).await {
        Ok(replies) => replies,
        Err(err) => {
            for entry in batch {
                fail(entry.event_id);
            }
            return Err(err);
        }
    };
    let mut summaries = Vec::new();
    for entry in batch {
        let reply = replies.get(&entry.event_id.to_string());
        match reply.and_then(Value::as_str).map(str::trim) {
            Some(summary) if !summary.is_empty() => {
                summaries.push((entry.event_id, summary.to_owned()));
            }
            _ => fail(entry.event_id),
        }
    }
    let db_path = db.to_owned();
    tokio::task::spawn_blocking(move || Store::open(&db_path)?.add_summaries(&summaries)).await??;
    Ok(true)
}

/// Whether `event` is one half of a tool call (Pre/Post/PostFailure with a `tool_use_id`).
fn is_tool_event(event: &Value) -> bool {
    let has_id = event
        .get("tool_use_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty());
    has_id
        && event
            .get("event")
            .and_then(Value::as_str)
            .is_some_and(|name| name.contains("ToolUse"))
}

/// `value` as text, cut to [`FIELD_CHARS`] characters with the full length noted.
fn short(value: Option<&Value>) -> Value {
    let text = match value {
        None | Some(Value::Null) => return Value::Null,
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    };
    let length = text.chars().count();
    if length <= FIELD_CHARS {
        return Value::String(text);
    }
    let head: String = text.chars().take(FIELD_CHARS).collect();
    Value::String(format!("{head}… [{length} chars]"))
}

/// Finished entries of a session's events in log order. A tool call's Pre and Post events
/// (paired by `tool_use_id`) form one entry; calls still running are left out.
fn read_entries(rows: &[Row]) -> Vec<Entry> {
    let mut entries: Vec<(bool, i64, Map<String, Value>)> = Vec::new();
    let mut index_by_tool_use: HashMap<String, usize> = HashMap::new();
    for (row, event) in rows.iter().filter_map(|row| {
        serde_json::from_str::<Value>(&row.json)
            .ok()
            .map(|event| (row, event))
    }) {
        if event.get("event").and_then(Value::as_str) == Some(ASSISTANT_MESSAGE) {
            continue;
        }
        let agent = event
            .get("agent_type")
            .filter(|v| !v.is_null())
            .or_else(|| event.get("agent_id"))
            .cloned()
            .unwrap_or(Value::Null);
        let field = |name: &str| event.get(name).cloned().unwrap_or(Value::Null);
        if is_tool_event(&event) {
            let tool_use_id = event["tool_use_id"].as_str().unwrap_or_default().to_owned();
            let index = *index_by_tool_use.entry(tool_use_id).or_insert_with(|| {
                let mut entry = Map::new();
                entry.insert("agent".into(), agent);
                entry.insert("tool".into(), field("tool_name"));
                entry.insert("input".into(), short(event.get("input")));
                entries.push((false, row.id, entry));
                entries.len() - 1
            });
            if event.get("event").and_then(Value::as_str) != Some("PreToolUse") {
                let (done, _, entry) = &mut entries[index];
                *done = true;
                entry.insert("status".into(), field("status"));
                entry.insert("duration_ms".into(), field("duration_ms"));
                entry.insert("response".into(), short(event.get("response")));
                entry.insert("error".into(), short(event.get("error")));
            }
        } else {
            let mut entry = Map::new();
            entry.insert("agent".into(), agent);
            entry.insert("event".into(), field("event"));
            entry.insert("prompt".into(), short(event.get("prompt")));
            entry.insert(
                "last_message".into(),
                short(event.pointer("/extra/last_assistant_message")),
            );
            entries.push((true, row.id, entry));
        }
    }
    entries
        .into_iter()
        .filter(|(done, _, _)| *done)
        .map(|(_, event_id, mut fields)| {
            fields.retain(|_, value| !value.is_null());
            fields.insert("key".into(), event_id.to_string().into());
            Entry {
                event_id,
                fields: Value::Object(fields),
            }
        })
        .collect()
}

/// Ask the model for one sentence per entry; returns its JSON object of key -> sentence.
async fn ask(llm: &Llm, batch: &[&Value]) -> Result<Map<String, Value>, SummarizeError> {
    let prompt = format!(
        "Summarize these log entries:\n{}",
        serde_json::to_string_pretty(batch)?
    );
    parse_reply(&llm.complete(SYSTEM_PROMPT, &prompt).await?)
}

/// The outermost JSON object in `text`, tolerating code fences or stray words around it.
fn parse_reply(text: &str) -> Result<Map<String, Value>, SummarizeError> {
    let object = text
        .find('{')
        .zip(text.rfind('}'))
        .filter(|(start, end)| start < end)
        .map(|(start, end)| &text[start..=end])
        .ok_or_else(|| SummarizeError::NoJson(text.chars().take(300).collect()))?;
    Ok(serde_json::from_str(object)?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn ask_sends_the_batch_and_parses_the_reply() {
        let llm = Llm::new(r#"cat >/dev/null; echo '{"1": "Listed files."}'"#);
        let entry = json!({"key": "1", "tool": "Bash"});
        let reply = ask(&llm, &[&entry]).await.unwrap();
        assert_eq!(reply.get("1"), Some(&json!("Listed files.")));
    }

    #[test]
    fn read_entries_pairs_tool_calls_and_skips_running_ones() {
        let log = [
            r#"{"event":"UserPromptSubmit","epoch_ms":1,"agent_id":"main","agent_type":"main","prompt":"hi"}"#,
            r#"{"event":"PreToolUse","tool_use_id":"t1","agent_id":"main","agent_type":"main","tool_name":"Bash","input":{"command":"ls"}}"#,
            r#"{"event":"PreToolUse","tool_use_id":"t2","agent_id":"a1","agent_type":null,"tool_name":"Read"}"#,
            r#"{"event":"PostToolUse","tool_use_id":"t1","status":"ok","duration_ms":5,"response":"x"}"#,
            r#"{"event":"AssistantMessage","message_id":"m1","texts":["thinking out loud"]}"#,
            "not json",
        ]
        .iter()
        .zip(1..)
        .map(|(json, id)| Row {
            id,
            json: (*json).to_owned(),
        })
        .collect::<Vec<_>>();

        let entries = read_entries(&log);
        assert_eq!(
            entries,
            vec![
                Entry {
                    event_id: 1,
                    fields: json!({"key": "1", "agent": "main", "event": "UserPromptSubmit", "prompt": "hi"}),
                },
                Entry {
                    event_id: 2,
                    fields: json!({"key": "2", "agent": "main", "tool": "Bash", "input": "{\"command\":\"ls\"}", "status": "ok", "duration_ms": 5, "response": "x"}),
                },
            ]
        );
    }

    #[test]
    fn short_cuts_long_text_and_notes_its_length() {
        let long = "x".repeat(FIELD_CHARS + 1);
        let cut = short(Some(&json!(long)));
        assert!(
            cut.as_str()
                .unwrap()
                .ends_with(&format!("… [{} chars]", FIELD_CHARS + 1))
        );
        assert_eq!(short(None), Value::Null);
    }

    #[test]
    fn parse_reply_tolerates_fences() {
        let reply = parse_reply("```json\n{\"t1\": \"Listed files.\"}\n```").unwrap();
        assert_eq!(reply.get("t1"), Some(&json!("Listed files.")));
        assert!(matches!(
            parse_reply("no json here"),
            Err(SummarizeError::NoJson(_))
        ));
    }
}
