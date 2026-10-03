//! One session's data as it arrives from a [`Feed`](crate::feed::Feed): events in time order,
//! summaries by entry key, and each model step merged from its `AssistantMessage` events.
//!
//! It only stores; [`model`](crate::model) interprets.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::call::{Key, key_of};
use crate::feed::Item;
use crate::step::Step;
use ichnos_core::event::ASSISTANT_MESSAGE;
use ichnos_core::event::Event;
use ichnos_core::store::Row;

/// A summary row: `{"event_id", "summary"}`.
#[derive(Debug, Deserialize)]
struct SummaryRow {
    event_id: i64,
    summary: String,
}

/// Everything received for one session so far.
#[derive(Debug, Clone, Default)]
pub struct SessionData {
    /// Sorted by `epoch_ms`.
    events: Vec<Arc<Event>>,
    /// Entry key -> summary sentence.
    summaries: HashMap<Key, String>,
    /// Event row id -> entry key; summaries name the row.
    key_by_row: HashMap<i64, Key>,
    /// In arrival order; a later piece of a step is merged into it in place.
    steps: Vec<Arc<Step>>,
    step_index: HashMap<String, usize>,
}

impl SessionData {
    /// The events, sorted by time.
    #[must_use]
    pub fn events(&self) -> &[Arc<Event>] {
        &self.events
    }

    /// The model steps, in arrival order.
    #[must_use]
    pub fn steps(&self) -> &[Arc<Step>] {
        &self.steps
    }

    /// The summary sentences, by entry key.
    #[must_use]
    pub fn summaries(&self) -> &HashMap<Key, String> {
        &self.summaries
    }

    /// Whether nothing arrived yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty() && self.steps.is_empty()
    }

    /// Store one item from a feed.
    ///
    /// # Errors
    ///
    /// Returns the [`serde_json::Error`] when an event or summary row can't be parsed; the item
    /// is then dropped.
    pub fn apply(&mut self, item: Item) -> Result<(), serde_json::Error> {
        match item {
            Item::Event(row) => {
                let event = Event::from_row(&row)?;
                if event.name() == ASSISTANT_MESSAGE {
                    self.add_step(Step::from_record(&row.json)?);
                } else {
                    self.add_event(event);
                }
            }
            Item::Summary(row) => {
                self.add_summary_row(&row)?;
            }
        }
        Ok(())
    }

    /// Add an event, keeping time order: the hook appends in order, but parallel tool calls can
    /// land a few milliseconds out of order.
    pub fn add_event(&mut self, event: Event) {
        self.key_by_row.insert(event.row_id(), key_of(&event));
        let at = self
            .events
            .partition_point(|other| other.epoch_ms() <= event.epoch_ms());
        self.events.insert(at, Arc::new(event));
    }

    /// File a summary under the entry of the event row it names; `false` when that event
    /// hasn't arrived.
    pub fn add_summary(&mut self, event_row: i64, summary: String) -> bool {
        let Some(key) = self.key_by_row.get(&event_row) else {
            return false;
        };
        self.summaries.insert(key.clone(), summary);
        true
    }

    fn add_summary_row(&mut self, row: &Row) -> Result<bool, serde_json::Error> {
        let SummaryRow { event_id, summary } = serde_json::from_str(&row.json)?;
        Ok(self.add_summary(event_id, summary))
    }

    /// Add a model step, or merge it into the earlier pieces of the same message.
    pub fn add_step(&mut self, step: Step) {
        if let Some(&index) = self.step_index.get(step.key()) {
            Arc::make_mut(&mut self.steps[index]).merge(step);
        } else {
            self.step_index
                .insert(step.key().to_owned(), self.steps.len());
            self.steps.push(Arc::new(step));
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn event(row_id: i64, record: &Value) -> Event {
        Event::parse(row_id, &record.to_string()).unwrap()
    }

    #[test]
    fn events_stay_sorted_when_parallel_calls_arrive_out_of_order() {
        let mut data = SessionData::default();
        for (row, ms, id) in [(1, 10, "a"), (2, 30, "b"), (3, 20, "c")] {
            data.add_event(event(
                row,
                &json!({"event": "PreToolUse", "epoch_ms": ms, "tool_use_id": id}),
            ));
        }
        let ids: Vec<_> = data.events().iter().map(|e| e.tool_use_id()).collect();
        assert_eq!(ids, [Some("a"), Some("c"), Some("b")]);
    }

    #[test]
    fn a_summary_is_filed_under_the_entry_of_the_event_row_it_names() {
        let mut data = SessionData::default();
        data.add_event(event(
            7,
            &json!({"event": "PreToolUse", "epoch_ms": 1, "tool_use_id": "t1"}),
        ));
        data.add_event(event(
            8,
            &json!({"event": "Stop", "epoch_ms": 2, "agent_id": "main"}),
        ));
        assert!(data.add_summary(7, "Listed files.".into()));
        assert!(data.add_summary(8, "Finished.".into()));
        assert_eq!(data.summaries()[&Key::from("t1")], "Listed files.");
        assert_eq!(data.summaries()[&Key::from("Stop:2:main")], "Finished.");
        assert!(!data.add_summary(99, "Unknown row.".into()));
    }

    #[test]
    fn assistant_messages_become_steps_merged_by_message() {
        let mut data = SessionData::default();
        for (id, text) in [(1, "hello"), (2, "again")] {
            let json = json!({"event": "AssistantMessage", "epoch_ms": id, "agent_id": "main",
                              "message_id": "m1", "texts": [text]});
            data.apply(Item::Event(Row {
                id,
                json: json.to_string(),
            }))
            .unwrap();
        }
        assert!(
            data.events().is_empty(),
            "model messages are not timeline events"
        );
        assert_eq!(data.steps().len(), 1);
        assert_eq!(data.steps()[0].texts(), ["hello", "again"]);
        assert_eq!(data.steps()[0].key(), "model:main:m1");
    }

    #[test]
    fn summary_rows_from_a_feed_are_parsed() {
        let mut data = SessionData::default();
        data.apply(Item::Event(Row {
            id: 3,
            json: r#"{"event":"Stop","epoch_ms":2}"#.into(),
        }))
        .unwrap();
        data.apply(Item::Summary(Row {
            id: 1,
            json: r#"{"event_id":3,"summary":"Done."}"#.into(),
        }))
        .unwrap();
        assert_eq!(data.summaries()[&Key::from("Stop:2:main")], "Done.");
        assert!(
            data.apply(Item::Summary(Row {
                id: 2,
                json: "{broken".into()
            }))
            .is_err()
        );
    }
}
