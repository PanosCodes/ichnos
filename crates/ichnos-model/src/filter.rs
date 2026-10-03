//! The filter box: free words plus `is:`, `tool:` (or `kind:`), `agent:` and `file:` terms, and
//! which rows a filter keeps.

use std::collections::HashSet;

use serde_json::Value;

use crate::call::{Call, Key};
use crate::labels::{Summaries, label};
use crate::model::{Node, NodeId, NodeKind, SessionModel};
use ichnos_core::event::{MAIN_AGENT, Status};

/// A parsed filter query. Values are lowercased.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    words: Vec<String>,
    /// `is:` terms: `error`, `running`, `ok`, `model`, `agent`; others are ignored.
    is: Vec<String>,
    tools: Vec<String>,
    agents: Vec<String>,
    files: Vec<String>,
}

impl Filter {
    /// Parse a query such as `is:error tool:Bash file:feed.rs boom`.
    #[must_use]
    pub fn parse(query: &str) -> Self {
        let mut filter = Self::default();
        for term in query.split_whitespace() {
            let parsed = term.split_once(':').filter(|(_, value)| !value.is_empty());
            let Some((name, value)) = parsed else {
                filter.words.push(term.to_lowercase());
                continue;
            };
            let value = value.to_lowercase();
            match name.to_lowercase().as_str() {
                "is" => filter.add_is(value),
                "tool" | "kind" => filter.tools.push(value),
                "agent" => filter.agents.push(value),
                "file" => filter.files.push(value),
                _ => filter.words.push(term.to_lowercase()),
            }
        }
        filter
    }

    /// The same filter, also keeping only failed calls.
    #[must_use]
    pub fn errors_only(mut self) -> Self {
        self.add_is("error".to_owned());
        self
    }

    fn add_is(&mut self, value: String) {
        if !self.is.contains(&value) {
            self.is.push(value);
        }
    }

    /// Whether the filter hides anything.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.words.is_empty() || self.is_structured()
    }

    /// Whether it has `is:`, `tool:`, `agent:` or `file:` terms.
    fn is_structured(&self) -> bool {
        !(self.is.is_empty()
            && self.tools.is_empty()
            && self.agents.is_empty()
            && self.files.is_empty())
    }

    /// Whether `node` itself matches.
    #[must_use]
    pub fn matches(&self, node: &Node, summaries: &Summaries, model: &SessionModel) -> bool {
        if !self.is.iter().all(|term| is_term(term, node)) {
            return false;
        }
        let label = label(node, summaries, model);
        if !self.tools.is_empty() {
            let names: Vec<String> = node
                .call()
                .and_then(|call| call.tool_name())
                .into_iter()
                .chain([label.chip.as_str()])
                .map(str::to_lowercase)
                .collect();
            if !self.tools.iter().any(|tool| names.contains(tool)) {
                return false;
            }
        }
        if !self.agents.is_empty() {
            let agent = agent_text(node);
            if !self.agents.iter().any(|term| agent.contains(term.as_str())) {
                return false;
            }
        }
        if !self.files.is_empty() {
            let files = file_text(node);
            if !self.files.iter().any(|term| files.contains(term.as_str())) {
                return false;
            }
        }
        if self.words.is_empty() {
            return true;
        }
        let haystack = format!(
            "{} {} {} {}",
            label.text,
            label.chip,
            label.sub.as_deref().unwrap_or_default(),
            raw_text(node)
        )
        .to_lowercase();
        self.words
            .iter()
            .all(|word| haystack.contains(word.as_str()))
    }

    /// Keys of the nodes to show: every match plus its ancestors.
    ///
    /// A turn is a container: it matches by its own words only, never by structured terms.
    #[must_use]
    pub fn visible_keys(&self, model: &SessionModel, summaries: &Summaries) -> HashSet<Key> {
        let mut keep = HashSet::new();
        for &turn in model.turns() {
            self.walk(turn, model, summaries, &mut keep);
        }
        keep
    }

    fn walk(
        &self,
        id: NodeId,
        model: &SessionModel,
        summaries: &Summaries,
        keep: &mut HashSet<Key>,
    ) -> bool {
        let node = model.node(id);
        let mut any =
            !(node.is_turn() && self.is_structured()) && self.matches(node, summaries, model);
        for &kid in node.children() {
            any |= self.walk(kid, model, summaries, keep);
        }
        if any {
            keep.insert(node.key().clone());
        }
        any
    }
}

/// Whether `node` passes one `is:` term; unknown terms pass.
fn is_term(term: &str, node: &Node) -> bool {
    match term {
        "error" => node.status() == Some(Status::Error),
        "running" => node.status() == Some(Status::Started),
        "ok" => node.status() == Some(Status::Ok),
        "model" => matches!(node.kind(), NodeKind::Model(_)),
        "agent" => node.is_subagent(),
        _ => true,
    }
}

/// Which agent a node belongs to, as searchable text: its type and id.
fn agent_text(node: &Node) -> String {
    let parts: Vec<&str> = match node.kind() {
        NodeKind::Agent(agent)
        | NodeKind::Call {
            agent: Some(agent), ..
        } => agent
            .agent_type()
            .into_iter()
            .chain([agent.agent_id()])
            .collect(),
        NodeKind::Call { call, .. } | NodeKind::Event(call) => call
            .agent_type()
            .into_iter()
            .chain([call.agent_id()])
            .collect(),
        NodeKind::Model(model) => vec![model.step().agent_id()],
        NodeKind::Turn { .. } | NodeKind::Note(_) => vec![MAIN_AGENT],
    };
    parts.join(" ").to_lowercase()
}

/// Where a call points: its path, command or pattern, as searchable text.
fn file_text(node: &Node) -> String {
    let Some(input) = node.call().map(Call::input) else {
        return String::new();
    };
    ["file_path", "notebook_path", "path", "command", "pattern"]
        .iter()
        .filter_map(|&name| input.get(name).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The raw records behind a node, as one searchable string.
fn raw_text(node: &Node) -> String {
    node.raw_data()
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labels::tests::label_events;
    use crate::model::build_session;
    use crate::model::tests::{DONE, data};

    const TURN: &str = "turn:UserPromptSubmit:1:main";

    fn keys(filter: &Filter) -> Vec<String> {
        let model = build_session(&data(&label_events(), &[]), DONE);
        let mut keys: Vec<String> = filter
            .visible_keys(&model, &Summaries::new())
            .into_iter()
            .map(|key| key.to_string())
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn filters_keep_matching_rows_and_their_ancestors() {
        assert_eq!(keys(&Filter::parse("main.rs")), ["t2", TURN]);
        assert_eq!(keys(&Filter::parse("").errors_only()), ["t1", TURN]);
    }

    #[test]
    fn the_filter_box_understands_is_tool_kind_agent_and_file_terms() {
        let filter = Filter::parse("  is:error Kind:Bash file:feed.rs  boom ");
        assert_eq!(
            (filter.is, filter.tools, filter.files, filter.words),
            (
                vec!["error".to_owned()],
                vec!["bash".to_owned()],
                vec!["feed.rs".to_owned()],
                vec!["boom".to_owned()]
            )
        );

        let keys = |query: &str| keys(&Filter::parse(query));
        assert_eq!(keys("is:error"), ["t1", TURN]);
        assert_eq!(keys("tool:read"), ["t2", TURN]);
        assert_eq!(keys("file:main.rs"), ["t2", TURN]);
        assert_eq!(keys("tool:bash cargo"), ["t1", TURN]);
        assert_eq!(keys("agent:main tool:read"), ["t2", TURN]);
        assert!(keys("tool:edit").is_empty(), "no match hides the turn too");
        assert_eq!(
            keys("fix the bug"),
            [TURN],
            "a turn still matches its own prompt"
        );
    }

    #[test]
    fn an_empty_filter_is_inactive() {
        assert!(!Filter::parse("  ").is_active());
        assert!(Filter::parse("x").is_active());
        assert!(Filter::parse("").errors_only().is_active());
    }
}
