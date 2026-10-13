//! The session model: a session's events and model steps as the tree the viewer shows, plus the
//! per-agent and per-moment numbers its views need.
//!
//! Pure: no clock, no I/O. Everything time-dependent comes in through a [`Moment`].
//!
//! The tree has one turn per prompt. A turn holds model steps and calls; a subagent's work hangs
//! under the call whose `spawned_agent` names it, or under the turn it ran in otherwise. Nodes live in one arena and refer to each other by [`NodeId`].

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::call::{Call, Key};
use crate::step::Step;
use ichnos_core::event::{Event, Status};

/// A call still "started" after this long without new events is not running any more.
pub const STALE_MS: i64 = 30 * 60 * 1000;

/// The moment a model is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    /// The replay cursor in epoch milliseconds, or `None` for now.
    pub at: Option<i64>,
    /// Whether the session is being followed live, so a running call keeps growing.
    pub live: bool,
    /// The current wall-clock time in epoch milliseconds.
    pub clock: i64,
}

/// The first and last known moments of a session, in epoch milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// The first moment.
    pub t0: i64,
    /// The last moment.
    pub t1: i64,
}

/// The session's first and last moments over everything received; `None` when empty.
#[must_use]
pub fn session_bounds(events: &[Arc<Event>], steps: &[Arc<Step>]) -> Option<Bounds> {
    let ends = events
        .first()
        .zip(events.last())
        .map(|(first, last)| (first.epoch_ms(), last.epoch_ms()));
    steps
        .iter()
        .map(|step| (step.first_ms(), step.last_ms()))
        .chain(ends)
        .reduce(|(t0, t1), (a, b)| (t0.min(a), t1.max(b)))
        .map(|(t0, t1)| Bounds { t0, t1 })
}

/// Where a still-running node ends at `moment`: the replay cursor, the wall clock while live and
/// recently active, else the last thing that happened.
#[must_use]
pub fn open_end(bounds: Option<Bounds>, moment: Moment) -> i64 {
    if let Some(at) = moment.at {
        return at;
    }
    let last = bounds.map_or(0, |bounds| bounds.t1);
    if moment.live && moment.clock - last < STALE_MS {
        moment.clock
    } else {
        last
    }
}

/// A node's index in its [`SessionModel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(usize);

impl NodeId {
    /// The node's position in its model's arena; stable for one model only.
    #[must_use]
    pub fn index(self) -> usize {
        self.0
    }
}

/// A subagent: from its `SubagentStart` and `SubagentStop` events.
#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    id: String,
    kind: Option<String>,
    start: Option<i64>,
    end: Option<i64>,
    start_event: Option<Call>,
    stop_event: Option<Call>,
}

impl Agent {
    fn new(agent_id: &str) -> Self {
        Self {
            id: agent_id.to_owned(),
            kind: None,
            start: None,
            end: None,
            start_event: None,
            stop_event: None,
        }
    }

    /// The subagent's id.
    #[must_use]
    pub fn agent_id(&self) -> &str {
        &self.id
    }

    /// The subagent's type, such as `Explore`.
    #[must_use]
    pub fn agent_type(&self) -> Option<&str> {
        self.kind.as_deref()
    }

    /// Its `SubagentStart` event.
    #[must_use]
    pub fn start_event(&self) -> Option<&Call> {
        self.start_event.as_ref()
    }

    /// Its `SubagentStop` event.
    #[must_use]
    pub fn stop_event(&self) -> Option<&Call> {
        self.stop_event.as_ref()
    }
}

/// A model step as of the model's moment: cut off at a replay cursor that falls inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelStep {
    step: Arc<Step>,
    last_ms: i64,
    partial: bool,
}

impl ModelStep {
    /// The model step.
    #[must_use]
    pub fn step(&self) -> &Step {
        &self.step
    }

    /// When its latest block, up to the model's moment, was written.
    #[must_use]
    pub fn last_ms(&self) -> i64 {
        self.last_ms
    }
}

/// What a node is.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// One prompt and everything until the next; the first turn may have no prompt.
    Turn {
        /// The `UserPromptSubmit` entry.
        prompt: Option<Call>,
        /// The `Stop` entry that ended it.
        stop: Option<Call>,
    },
    /// A tool call; one that started a subagent carries it, and its children are the subagent's.
    Call {
        /// The call.
        call: Call,
        /// The subagent it started.
        agent: Option<Agent>,
    },
    /// Any other hook event.
    Event(Call),
    /// A subagent no `Agent` call is known to have started.
    Agent(Agent),
    /// One of the agent's own one-line progress notes: an untyped subagent with no calls.
    Note(Agent),
    /// One model message.
    Model(ModelStep),
}

/// Time spent and tokens used under a node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    /// Milliseconds the model spent writing.
    pub model_ms: i64,
    /// Milliseconds spent in tools.
    pub tool_ms: i64,
    /// Tokens the model wrote.
    pub output_tokens: i64,
    /// Tokens the model read.
    pub input_tokens: i64,
}

/// One row of the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    key: Key,
    kind: NodeKind,
    parent: Option<NodeId>,
    depth: usize,
    children: Vec<NodeId>,
    start: Option<i64>,
    end: Option<i64>,
    status: Option<Status>,
    totals: Totals,
    errors: usize,
    running: bool,
}

impl Node {
    fn new(key: Key, kind: NodeKind, start: Option<i64>) -> Self {
        Self {
            key,
            kind,
            parent: None,
            depth: 0,
            children: Vec::new(),
            start,
            end: None,
            status: None,
            totals: Totals::default(),
            errors: 0,
            running: false,
        }
    }

    /// The node's key; stable across rebuilds.
    #[must_use]
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// What the node is.
    #[must_use]
    pub fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// The database row id of the event that names the node: a call's first event, a turn's
    /// prompt (its `Stop` when it has none), a subagent's start or stop event. `None` for a
    /// model step, which is made of several events.
    #[must_use]
    pub fn row_id(&self) -> Option<i64> {
        let call = match &self.kind {
            NodeKind::Turn { prompt, stop } => prompt.as_ref().or(stop.as_ref()),
            NodeKind::Call { call, .. } | NodeKind::Event(call) => Some(call),
            NodeKind::Agent(agent) | NodeKind::Note(agent) => {
                agent.start_event().or(agent.stop_event())
            }
            NodeKind::Model(_) => None,
        };
        call.map(|call| call.first().row_id())
    }

    /// The node it hangs under; `None` for a turn.
    #[must_use]
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// How deep it hangs: 0 for a turn.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// The nodes under it, by start time.
    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    /// When it started.
    #[must_use]
    pub fn start(&self) -> Option<i64> {
        self.start
    }

    /// When it ended; `None` while running.
    #[must_use]
    pub fn end(&self) -> Option<i64> {
        self.end
    }

    /// How it stands; `None` for nodes without one, such as finished model steps.
    #[must_use]
    pub fn status(&self) -> Option<Status> {
        self.status
    }

    /// Time and tokens under it.
    #[must_use]
    pub fn totals(&self) -> Totals {
        self.totals
    }

    /// Failed calls under it.
    #[must_use]
    pub fn errors(&self) -> usize {
        self.errors
    }

    /// Whether it, or anything under it, is still running.
    #[must_use]
    pub fn running(&self) -> bool {
        self.running
    }

    /// The call or event entry behind a call or event node.
    #[must_use]
    pub fn call(&self) -> Option<&Call> {
        match &self.kind {
            NodeKind::Call { call, .. } | NodeKind::Event(call) => Some(call),
            _ => None,
        }
    }

    /// The subagent of an agent or note node, or the one a call started.
    #[must_use]
    pub fn agent(&self) -> Option<&Agent> {
        match &self.kind {
            NodeKind::Agent(agent) | NodeKind::Note(agent) => Some(agent),
            NodeKind::Call { agent, .. } => agent.as_ref(),
            _ => None,
        }
    }

    /// The model step of a model node.
    #[must_use]
    pub fn model_step(&self) -> Option<&ModelStep> {
        match &self.kind {
            NodeKind::Model(step) => Some(step),
            _ => None,
        }
    }

    /// Whether this is a turn.
    #[must_use]
    pub fn is_turn(&self) -> bool {
        matches!(self.kind, NodeKind::Turn { .. })
    }

    /// Whether this is a subagent: an agent node, or a call that started one.
    #[must_use]
    pub fn is_subagent(&self) -> bool {
        matches!(
            self.kind,
            NodeKind::Agent(_) | NodeKind::Call { agent: Some(_), .. }
        )
    }

    /// The raw records behind the node, for "copy raw data as JSON": its events, or its step.
    #[must_use]
    pub fn raw_data(&self) -> Vec<Value> {
        if let NodeKind::Model(model) = &self.kind {
            return vec![model.step.to_json()];
        }
        let calls: Vec<&Call> = match &self.kind {
            NodeKind::Turn { prompt, stop } => prompt.iter().chain(stop).collect(),
            NodeKind::Call { call, agent } => std::iter::once(call)
                .chain(
                    agent
                        .iter()
                        .flat_map(|a| a.start_event.iter().chain(&a.stop_event)),
                )
                .collect(),
            NodeKind::Event(call) => vec![call],
            NodeKind::Agent(agent) | NodeKind::Note(agent) => {
                agent.start_event.iter().chain(&agent.stop_event).collect()
            }
            NodeKind::Model(_) => Vec::new(),
        };
        calls
            .into_iter()
            .flat_map(Call::events)
            .map(|event| serde_json::from_str(event.json()).unwrap_or(Value::Null))
            .collect()
    }
}

/// A session as a tree at one moment.
#[derive(Debug, Clone, Default)]
pub struct SessionModel {
    nodes: Vec<Node>,
    turns: Vec<NodeId>,
    by_key: HashMap<Key, NodeId>,
    /// Every node in the tree, depth first.
    order: Vec<NodeId>,
    calls: Vec<Call>,
    end: i64,
}

impl SessionModel {
    /// The turns, in time order.
    #[must_use]
    pub fn turns(&self) -> &[NodeId] {
        &self.turns
    }

    /// The node `id`.
    ///
    /// # Panics
    ///
    /// Panics when `id` comes from another model.
    #[must_use]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    /// The node with `key`, if the tree holds one.
    #[must_use]
    pub fn find(&self, key: &Key) -> Option<NodeId> {
        self.by_key.get(key).copied()
    }

    /// Every node in the tree, depth first.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.order.iter().map(|&id| (id, self.node(id)))
    }

    /// Every entry (calls and other events) up to the model's moment, in time order.
    #[must_use]
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    /// Where running nodes end at the model's moment.
    #[must_use]
    pub fn end(&self) -> i64 {
        self.end
    }

    /// The turn that `id` is in.
    #[must_use]
    pub fn turn_of(&self, mut id: NodeId) -> NodeId {
        while let Some(parent) = self.node(id).parent {
            id = parent;
        }
        id
    }
}

mod build;
#[cfg(test)]
pub(crate) mod tests;
mod views;

pub use build::build_session;
pub use views::{AgentBox, AgentGraph, FileTouch, StateAt, Stats, ToolTally, moments};
