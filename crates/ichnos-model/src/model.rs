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

use crate::call::{Call, Key, pair_calls};
use crate::session_data::SessionData;
use crate::step::Step;
use ichnos_core::event::{Event, FileOp, MAIN_AGENT, Status, Todo};

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

    /// The headline numbers for the whole session.
    #[must_use]
    pub fn stats(&self) -> Stats {
        let mut stats = Stats::default();
        for (_, node) in self.iter() {
            if node.is_subagent() {
                stats.subagents += 1;
            }
            if let NodeKind::Call { .. } = node.kind {
                stats.calls += 1;
                match node.status {
                    Some(Status::Started) => stats.running += 1,
                    Some(Status::Error) => stats.errors += 1,
                    _ => {}
                }
            }
        }
        for &id in &self.turns {
            let turn = self.node(id);
            if matches!(
                turn.kind,
                NodeKind::Turn {
                    prompt: Some(_),
                    ..
                }
            ) {
                stats.turns += 1;
            }
            stats.model_ms += turn.totals.model_ms;
            stats.output_tokens += turn.totals.output_tokens;
        }
        stats
    }

    /// One box per agent (main and each subagent) with per-tool counts and time, model time and
    /// tokens; `describe(node)` names a subagent's box.
    #[must_use]
    pub fn agent_graph(&self, describe: impl Fn(&Node) -> String) -> AgentGraph {
        let mut graph = AgentGraph {
            boxes: vec![AgentBox {
                id: MAIN_AGENT.to_owned(),
                agent_type: Some(MAIN_AGENT.to_owned()),
                what: "Main agent".to_owned(),
                key: None,
                start: self.turns.first().and_then(|&id| self.node(id).start),
                end: None,
                running: self.turns.iter().any(|&id| self.node(id).running),
                parent: None,
                tools: Vec::new(),
                model_ms: 0,
                output_tokens: 0,
                errors: 0,
                children: Vec::new(),
            }],
        };
        for (_, node) in self.iter() {
            let subagent = match &node.kind {
                NodeKind::Agent(agent) => Some((agent, MAIN_AGENT)),
                NodeKind::Call {
                    call,
                    agent: Some(agent),
                } => Some((agent, call.agent_id())),
                _ => None,
            };
            if let Some((agent, parent)) = subagent {
                let entry = graph.entry(&agent.id);
                entry.agent_type = Some(agent.agent_type().unwrap_or("subagent").to_owned());
                entry.what = describe(node);
                entry.key = Some(node.key.clone());
                entry.start = node.start;
                entry.end = node.end;
                entry.running = node.end.is_none();
                entry.parent = Some(parent.to_owned());
            }
            if let NodeKind::Call { call, .. } = &node.kind {
                let failed = node.status == Some(Status::Error);
                let entry = graph.entry(call.agent_id());
                let name = call.tool_name().unwrap_or_default();
                let at = entry
                    .tools
                    .iter()
                    .position(|(tool, _)| tool == name)
                    .unwrap_or_else(|| {
                        entry.tools.push((name.to_owned(), ToolTally::default()));
                        entry.tools.len() - 1
                    });
                let tool = &mut entry.tools[at].1;
                tool.calls += 1;
                if failed {
                    tool.errors += 1;
                    entry.errors += 1;
                }
                if let Some(end) = call.end() {
                    tool.ms += end - call.start();
                }
            }
            if let NodeKind::Model(model) = &node.kind {
                let ms = (node.end.unwrap_or(self.end) - node.start.unwrap_or(self.end)).max(0);
                let entry = graph.entry(model.step.agent_id());
                entry.model_ms += ms;
                entry.output_tokens += model.step.output_tokens();
            }
        }
        for index in 1..graph.boxes.len() {
            let parent = graph.boxes[index]
                .parent
                .as_deref()
                .and_then(|parent| graph.position(parent));
            if let Some(parent) = parent {
                graph.boxes[parent].children.push(index);
            }
        }
        let starts: Vec<i64> = graph.boxes.iter().map(|b| b.start.unwrap_or(0)).collect();
        for entry in &mut graph.boxes {
            entry.children.sort_by_key(|&child| starts[child]);
        }
        graph
    }

    /// What the session looked like at `at`, rebuilt from the calls made up to then.
    #[must_use]
    pub fn state_at(&self, at: i64) -> StateAt {
        let mut state = StateAt::default();
        for (id, node) in self.iter() {
            let started = node.start.is_some_and(|start| start <= at);
            let open = node.end.is_none_or(|end| end > at);
            let tracked = matches!(
                node.kind,
                NodeKind::Call { .. } | NodeKind::Model(_) | NodeKind::Agent(_)
            );
            if tracked && started && open {
                state.running.push(id);
            }
            if let NodeKind::Model(model) = &node.kind
                && started
            {
                state.output_tokens += model.step.output_tokens();
            }
            let NodeKind::Call { call, .. } = &node.kind else {
                continue;
            };
            if !started {
                continue;
            }
            state.calls += 1;
            let failed = node.status == Some(Status::Error);
            if failed {
                state.errors += 1;
            }
            for file in call.files() {
                let list = match file.op {
                    FileOp::Write => &mut state.changed,
                    FileOp::Read => &mut state.read,
                    FileOp::Other => continue,
                };
                let write = file.op == FileOp::Write;
                match list.iter_mut().find(|touch| touch.path == file.path) {
                    Some(touch) => {
                        touch.times += 1;
                        touch.failed = write && failed;
                    }
                    None => list.push(FileTouch {
                        path: file.path.clone(),
                        times: 1,
                        failed: write && failed,
                    }),
                }
            }
            if let Some(skill) = call.skill()
                && !state.skills.iter().any(|known| known == skill)
            {
                state.skills.push(skill.to_owned());
            }
            if let Some(todos) = call.todos() {
                state.todos = Some(todos.to_vec());
            }
        }
        state.turns = self
            .turns
            .iter()
            .map(|&id| self.node(id))
            .filter(|turn| {
                matches!(
                    turn.kind,
                    NodeKind::Turn {
                        prompt: Some(_),
                        ..
                    }
                ) && turn.start.is_some_and(|start| start <= at)
            })
            .count();
        state
    }
}

/// The headline numbers of a session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Prompts sent.
    pub turns: usize,
    /// Tool calls made.
    pub calls: usize,
    /// Subagents started.
    pub subagents: usize,
    /// Tool calls still running.
    pub running: usize,
    /// Tool calls that failed.
    pub errors: usize,
    /// Milliseconds the model spent writing.
    pub model_ms: i64,
    /// Tokens the model wrote.
    pub output_tokens: i64,
}

/// Calls of one tool by one agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolTally {
    /// Calls made.
    pub calls: usize,
    /// Calls that failed.
    pub errors: usize,
    /// Milliseconds spent in finished calls.
    pub ms: i64,
}

/// One agent's box in the [`AgentGraph`].
#[derive(Debug, Clone, PartialEq)]
pub struct AgentBox {
    id: String,
    agent_type: Option<String>,
    what: String,
    key: Option<Key>,
    start: Option<i64>,
    end: Option<i64>,
    running: bool,
    parent: Option<String>,
    tools: Vec<(String, ToolTally)>,
    model_ms: i64,
    output_tokens: i64,
    errors: usize,
    children: Vec<usize>,
}

impl AgentBox {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            agent_type: None,
            what: String::new(),
            key: None,
            start: None,
            end: None,
            running: false,
            parent: None,
            tools: Vec::new(),
            model_ms: 0,
            output_tokens: 0,
            errors: 0,
            children: Vec::new(),
        }
    }

    /// The agent's id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The agent's type: `main`, `Explore`, `subagent`, …
    #[must_use]
    pub fn agent_type(&self) -> Option<&str> {
        self.agent_type.as_deref()
    }

    /// What the agent was asked to do.
    #[must_use]
    pub fn what(&self) -> &str {
        &self.what
    }

    /// The tree node that started it; `None` for the main agent.
    #[must_use]
    pub fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// When it started.
    #[must_use]
    pub fn start(&self) -> Option<i64> {
        self.start
    }

    /// When it ended.
    #[must_use]
    pub fn end(&self) -> Option<i64> {
        self.end
    }

    /// Whether it still runs.
    #[must_use]
    pub fn running(&self) -> bool {
        self.running
    }

    /// Its tools in first-use order, with counts and time.
    #[must_use]
    pub fn tools(&self) -> &[(String, ToolTally)] {
        &self.tools
    }

    /// Milliseconds its model spent writing.
    #[must_use]
    pub fn model_ms(&self) -> i64 {
        self.model_ms
    }

    /// Tokens its model wrote.
    #[must_use]
    pub fn output_tokens(&self) -> i64 {
        self.output_tokens
    }

    /// Its failed calls.
    #[must_use]
    pub fn errors(&self) -> usize {
        self.errors
    }

    /// The boxes of the subagents it started, as indexes into the graph, by start time.
    #[must_use]
    pub fn children(&self) -> &[usize] {
        &self.children
    }
}

/// The agents of a session as boxes: the main agent first, subagents under who started them.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentGraph {
    boxes: Vec<AgentBox>,
}

impl AgentGraph {
    /// The main agent's box.
    #[must_use]
    pub fn main(&self) -> &AgentBox {
        &self.boxes[0]
    }

    /// The box at `index`, as listed in [`AgentBox::children`].
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&AgentBox> {
        self.boxes.get(index)
    }

    /// The box of agent `id`.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&AgentBox> {
        self.position(id).map(|index| &self.boxes[index])
    }

    /// Every box, the main agent first.
    pub fn iter(&self) -> impl Iterator<Item = &AgentBox> {
        self.boxes.iter()
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.boxes.iter().position(|entry| entry.id == id)
    }

    fn entry(&mut self, id: &str) -> &mut AgentBox {
        let index = self.position(id).unwrap_or_else(|| {
            self.boxes.push(AgentBox::new(id));
            self.boxes.len() - 1
        });
        &mut self.boxes[index]
    }
}

/// A file a session wrote or read, and how often.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTouch {
    /// The file's path.
    pub path: String,
    /// How many calls touched it.
    pub times: usize,
    /// Whether the latest call that changed it failed.
    pub failed: bool,
}

/// What a session looked like at one moment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StateAt {
    /// Calls, model steps and subagents running then.
    pub running: Vec<NodeId>,
    /// The latest todo list a call set, if there was one yet.
    pub todos: Option<Vec<Todo>>,
    /// Files changed so far, in first-change order.
    pub changed: Vec<FileTouch>,
    /// Files read so far, in first-read order.
    pub read: Vec<FileTouch>,
    /// Skills loaded so far.
    pub skills: Vec<String>,
    /// Prompts sent so far.
    pub turns: usize,
    /// Calls made so far.
    pub calls: usize,
    /// Calls that failed so far.
    pub errors: usize,
    /// Tokens the model wrote so far.
    pub output_tokens: i64,
}

/// Every moment something happened, ascending, for stepping through a replay.
#[must_use]
pub fn moments(events: &[Arc<Event>], steps: &[Arc<Step>]) -> Vec<i64> {
    let mut all: Vec<i64> = events.iter().map(|event| event.epoch_ms()).collect();
    all.extend(
        steps
            .iter()
            .flat_map(|step| [step.first_ms(), step.last_ms()]),
    );
    all.sort_unstable();
    all.dedup();
    all
}

/// The session at `moment` as a tree.
#[must_use]
pub fn build_session(data: &SessionData, moment: Moment) -> SessionModel {
    build(data.events(), data.steps(), moment)
}

/// Builds the arena, one phase per method.
#[derive(Debug, Default)]
struct Builder {
    nodes: Vec<Node>,
    turns: Vec<NodeId>,
    agents: HashMap<String, NodeId>,
    /// Agent ids in first-seen order.
    agent_order: Vec<String>,
    /// Agent id -> the `Agent` call node that started it.
    agent_calls: HashMap<String, NodeId>,
    /// Agent id -> the node now holding the subagent's children: the call that started it, or
    /// its own agent node.
    home: HashMap<String, NodeId>,
}

impl Builder {
    fn push(&mut self, node: Node) -> NodeId {
        self.nodes.push(node);
        NodeId(self.nodes.len() - 1)
    }

    fn node(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0]
    }

    fn new_turn(&mut self, prompt: Option<Call>) -> NodeId {
        let key = prompt.as_ref().map_or_else(
            || Key::from("turn:start"),
            |call| call.key().prefixed("turn"),
        );
        let start = prompt.as_ref().map(Call::start);
        let id = self.push(Node::new(key, NodeKind::Turn { prompt, stop: None }, start));
        self.turns.push(id);
        id
    }

    fn agent_node(&mut self, agent_id: &str) -> NodeId {
        if let Some(&id) = self.agents.get(agent_id) {
            return id;
        }
        let key = Key::from(format!("agent:{agent_id}"));
        let id = self.push(Node::new(key, NodeKind::Agent(Agent::new(agent_id)), None));
        self.agents.insert(agent_id.to_owned(), id);
        self.agent_order.push(agent_id.to_owned());
        id
    }

    /// The subagent held by agent, note or `Agent` call node `id`.
    ///
    /// # Panics
    ///
    /// Panics when `id` holds no subagent: a bug in the builder.
    fn agent_mut(&mut self, id: NodeId) -> &mut Agent {
        match &mut self.node(id).kind {
            NodeKind::Agent(agent)
            | NodeKind::Note(agent)
            | NodeKind::Call {
                agent: Some(agent), ..
            } => agent,
            other => unreachable!("not an agent node: {other:?}"),
        }
    }

    /// The turn that was current at `ms`: the last one started by then, else the first.
    fn turn_at(&mut self, ms: i64) -> NodeId {
        if self.turns.is_empty() {
            self.new_turn(None);
        }
        let mut found = self.turns[0];
        for &turn in &self.turns {
            if self.nodes[turn.0].start.is_none_or(|start| start <= ms) {
                found = turn;
            }
        }
        found
    }

    /// Remember a call by the subagent it started.
    fn register_agent_call(&mut self, id: NodeId) {
        let Some(call) = self.nodes[id.0].call() else {
            return;
        };
        if let Some(agent_id) = call.spawned_agent() {
            self.agent_calls.insert(agent_id.to_owned(), id);
        }
    }

    /// Mark calls among `kids` that never finished as interrupted at `end`.
    fn close(&mut self, kids: &[NodeId], end: i64) {
        for &kid in kids {
            if let NodeKind::Call { call, agent: None } = &mut self.node(kid).kind
                && call.end().is_none()
            {
                call.interrupt(end);
            }
        }
    }

    /// Sort entries into turns (main agent) and agent nodes (subagents).
    fn add_calls(&mut self, calls: &[Call]) {
        let mut turn: Option<NodeId> = None;
        for call in calls {
            match call.name() {
                "SubagentStart" => {
                    let id = self.agent_node(call.agent_id());
                    let agent = self.agent_mut(id);
                    agent.start = Some(call.start());
                    agent.kind = call.agent_type().map(str::to_owned).or(agent.kind.take());
                    agent.start_event = Some(call.clone());
                    self.node(id).start = Some(call.start());
                    continue;
                }
                "SubagentStop" => {
                    let id = self.agent_node(call.agent_id());
                    let agent = self.agent_mut(id);
                    agent.end = Some(call.start());
                    if agent.kind.is_none() {
                        agent.kind = call.agent_type().map(str::to_owned);
                    }
                    agent.stop_event = Some(call.clone());
                    self.node(id).end = Some(call.start());
                    continue;
                }
                _ => {}
            }
            let kind = if call.tool_use_id().is_some() {
                NodeKind::Call {
                    call: call.clone(),
                    agent: None,
                }
            } else {
                NodeKind::Event(call.clone())
            };
            let node = Node::new(call.key().clone(), kind, Some(call.start()));
            let parent = if call.agent_id() == MAIN_AGENT {
                if call.name() == "UserPromptSubmit" {
                    turn = Some(self.new_turn(Some(call.clone())));
                    continue;
                }
                let current = *turn.get_or_insert_with(|| self.new_turn(None));
                if call.name() == "Stop" {
                    if let NodeKind::Turn { stop, .. } = &mut self.node(current).kind {
                        *stop = Some(call.clone());
                    }
                    continue;
                }
                current
            } else {
                self.agent_node(call.agent_id())
            };
            let id = self.push(node);
            self.register_agent_call(id);
            self.node(parent).children.push(id);
        }
    }

    /// Put each subagent under the call that started it, or into the turn it ran in.
    fn place_agents(&mut self) {
        for agent_id in self.agent_order.clone() {
            let id = self.agents[&agent_id];
            if let Some(&parent) = self.agent_calls.get(&agent_id) {
                let NodeKind::Agent(agent) = self.node(id).kind.clone() else {
                    unreachable!("agents start as agent nodes")
                };
                let children = std::mem::take(&mut self.node(id).children);
                let parent_node = self.node(parent);
                if let NodeKind::Call { agent: slot, .. } = &mut parent_node.kind {
                    *slot = Some(agent);
                }
                parent_node.children = children;
                self.home.insert(agent_id, parent);
            } else {
                let node = &self.nodes[id.0];
                let first_child = node
                    .children
                    .first()
                    .and_then(|&kid| self.nodes[kid.0].start);
                let ms = node.start.or(first_child).or(node.end).unwrap_or(0);
                let turn = self.turn_at(ms);
                self.node(turn).children.push(id);
                self.home.insert(agent_id, id);
            }
        }
    }

    /// A call with no Post event inside a turn or subagent that already ended was interrupted.
    /// A turn ends at its Stop event, or when the next prompt arrives.
    fn close_interrupted(&mut self) {
        for (i, id) in self.turns.clone().into_iter().enumerate() {
            let closed_at = match &self.nodes[id.0].kind {
                NodeKind::Turn {
                    stop: Some(stop), ..
                } => Some(stop.start()),
                _ => self
                    .turns
                    .get(i + 1)
                    .and_then(|&next| self.nodes[next.0].start),
            };
            if let Some(at) = closed_at {
                let kids = self.nodes[id.0].children.clone();
                self.close(&kids, at);
            }
        }
        for agent_id in self.agent_order.clone() {
            let id = self.home[&agent_id];
            let agent = self.agent_mut(id);
            if agent.stop_event.is_some()
                && let Some(at) = agent.end
            {
                let kids = self.nodes[id.0].children.clone();
                self.close(&kids, at);
            }
        }
    }

    /// Untyped subagents with no calls are the agent's own progress notes.
    fn mark_notes(&mut self) {
        for agent_id in self.agent_order.clone() {
            let id = self.agents[&agent_id];
            let started_by_call = self.agent_calls.contains_key(&agent_id);
            let node = self.node(id);
            if let NodeKind::Agent(agent) = &node.kind
                && agent.kind.is_none()
                && node.children.is_empty()
                && !started_by_call
            {
                node.kind = NodeKind::Note(agent.clone());
                node.start = node.start.or(node.end);
            }
        }
    }

    /// Put model steps into the turn or subagent that produced them. A step starts when its
    /// agent was last waiting on something: the prompt, a tool result, the previous step.
    fn add_steps(&mut self, steps: Vec<ModelStep>) {
        let mut containers: Vec<(NodeId, Option<i64>)> = Vec::new();
        for model in steps {
            let agent_id = model.step.agent_id().to_owned();
            let owner = (agent_id != MAIN_AGENT)
                .then(|| self.agents.get(&agent_id).copied())
                .flatten()
                .filter(|&owner| !matches!(self.nodes[owner.0].kind, NodeKind::Note(_)));
            let (container, container_start) = if let Some(owner) = owner {
                (self.home[&agent_id], self.nodes[owner.0].start)
            } else {
                let turn = self.turn_at(model.step.first_ms());
                (turn, self.nodes[turn.0].start)
            };
            let first_ms = model.step.first_ms();
            let model_end = (!model.partial).then_some(model.last_ms);
            let key = Key::from(model.step.key());
            let mut node = Node::new(key, NodeKind::Model(model), Some(first_ms));
            node.end = model_end;
            let id = self.push(node);
            self.node(container).children.push(id);
            if !containers.iter().any(|(known, _)| *known == container) {
                containers.push((container, container_start));
            }
        }
        for (container, container_start) in containers {
            self.start_steps(container, container_start);
        }
    }

    /// Start each model step under `container` when its agent was last waiting.
    fn start_steps(&mut self, container: NodeId, container_start: Option<i64>) {
        let kids = self.nodes[container.0].children.clone();
        let waits: Vec<i64> = kids
            .iter()
            .filter_map(|&kid| match &self.nodes[kid.0].kind {
                NodeKind::Call { call, .. } => call.end(),
                NodeKind::Event(_) => self.nodes[kid.0].start,
                _ => None,
            })
            .collect();
        let mut models: Vec<(NodeId, i64, i64)> = kids
            .into_iter()
            .filter_map(|kid| {
                let model = self.nodes[kid.0].model_step()?;
                Some((kid, model.step.first_ms(), model.last_ms))
            })
            .collect();
        models.sort_by_key(|&(_, first_ms, _)| first_ms);
        let mut previous = container_start;
        for (kid, first_ms, last_ms) in models {
            let mut from = previous.unwrap_or(first_ms);
            for &wait in &waits {
                if wait <= first_ms && wait > from {
                    from = wait;
                }
            }
            self.node(kid).start = Some(from.min(first_ms));
            previous = Some(last_ms);
        }
    }

    /// Settle every turn and what's under it, and hand the arena over as a model.
    fn finish_turns(mut self, calls: Vec<Call>, end: i64, moment: Moment) -> SessionModel {
        let mut model = SessionModel {
            nodes: Vec::new(),
            turns: self.turns.clone(),
            by_key: HashMap::new(),
            order: Vec::new(),
            calls,
            end,
        };
        let still_going = moment.live || moment.at.is_some();
        let last = self.turns.len().saturating_sub(1);
        for (i, turn) in self.turns.clone().into_iter().enumerate() {
            if self.nodes[turn.0].start.is_none() {
                let kids = &self.nodes[turn.0].children;
                let start = kids.iter().filter_map(|&kid| self.nodes[kid.0].start).min();
                self.node(turn).start = start;
            }
            finish(&mut self.nodes, turn, None, 0, end, &mut model);
            let node = &self.nodes[turn.0];
            let kid_ends: Vec<Option<i64>> = node
                .children
                .iter()
                .map(|&kid| self.nodes[kid.0].end)
                .collect();
            let max_kid_end = kid_ends.iter().flatten().copied().chain(node.start).max();
            let open_kid = kid_ends.contains(&None);
            let stop_start = match &node.kind {
                NodeKind::Turn {
                    stop: Some(stop), ..
                } => Some(stop.start()),
                _ => None,
            };
            let turn_end = if stop_start.is_some() {
                stop_start
            } else if i == last && (still_going || open_kid) {
                None
            } else {
                max_kid_end
            };
            let running =
                turn_end.is_none() || node.children.iter().any(|&kid| self.nodes[kid.0].running);
            let errors = node.errors;
            let node = self.node(turn);
            node.end = turn_end;
            node.running = running;
            node.status = Some(match (turn_end, errors) {
                (None, _) => Status::Started,
                (Some(_), 0) => Status::Ok,
                (Some(_), _) => Status::Error,
            });
        }
        model.nodes = self.nodes;
        model
    }
}

/// The events and steps as they were at the replay cursor `at` (all of them when `None`).
fn snapshot(
    events: &[Arc<Event>],
    steps: &[Arc<Step>],
    at: Option<i64>,
) -> (Vec<Arc<Event>>, Vec<ModelStep>) {
    let until = at.unwrap_or(i64::MAX);
    let events = events
        .iter()
        .filter(|event| event.epoch_ms() <= until)
        .cloned()
        .collect();
    let steps = steps
        .iter()
        .filter(|step| step.first_ms() <= until)
        .map(|step| ModelStep {
            step: Arc::clone(step),
            last_ms: step.last_ms().min(until),
            partial: step.last_ms() > until,
        })
        .collect();
    (events, steps)
}

fn build(all_events: &[Arc<Event>], all_steps: &[Arc<Step>], moment: Moment) -> SessionModel {
    let end = open_end(session_bounds(all_events, all_steps), moment);
    let (events, steps) = snapshot(all_events, all_steps, moment.at);
    let calls = pair_calls(&events);
    let mut builder = Builder::default();
    builder.add_calls(&calls);
    builder.place_agents();
    builder.close_interrupted();
    builder.mark_notes();
    builder.add_steps(steps);
    builder.finish_turns(calls, end, moment)
}

/// Settle `id` and everything under it: parent links, depth, child order, start and end,
/// status, totals and error counts.
fn finish(
    nodes: &mut [Node],
    id: NodeId,
    parent: Option<NodeId>,
    depth: usize,
    end: i64,
    model: &mut SessionModel,
) {
    {
        let node = &mut nodes[id.0];
        node.parent = parent;
        node.depth = depth;
    }
    model.by_key.insert(nodes[id.0].key.clone(), id);
    model.order.push(id);
    let mut kids = std::mem::take(&mut nodes[id.0].children);
    kids.sort_by_key(|&kid| nodes[kid.0].start.unwrap_or(0));
    for &kid in &kids {
        finish(nodes, kid, Some(id), depth + 1, end, model);
    }
    let first_kid_start = kids.first().and_then(|&kid| nodes[kid.0].start);

    let node = &mut nodes[id.0];
    match &node.kind {
        NodeKind::Call { call, agent } => {
            let agent_start = agent.as_ref().and_then(|agent| agent.start);
            node.start = Some(agent_start.map_or(call.start(), |a| a.min(call.start())));
            node.status = call.status();
            node.end = call.end();
            if let Some(agent) = agent {
                // Async subagents outlive their Agent call.
                node.end = agent
                    .stop_event
                    .as_ref()
                    .map(|_| call.end().unwrap_or(0).max(agent.end.unwrap_or(0)));
            }
            if node.end.is_none() {
                node.status = Some(Status::Started);
            }
        }
        NodeKind::Model(_) => {
            node.status = node.end.is_none().then_some(Status::Started);
        }
        NodeKind::Note(_) => {
            node.status = None;
            node.end = node.end.or(node.start);
        }
        NodeKind::Agent(agent) => {
            node.start = node.start.or(first_kid_start);
            if agent.stop_event.is_some() {
                node.status = Some(Status::Ok);
            } else {
                node.status = Some(Status::Started);
                node.end = None;
            }
        }
        NodeKind::Event(call) => node.end = call.end(),
        NodeKind::Turn { .. } => {}
    }

    let mut totals = Totals::default();
    let mut errors = 0;
    let mut kid_running = false;
    for &kid in &kids {
        let kid = &nodes[kid.0];
        totals.model_ms += kid.totals.model_ms;
        totals.tool_ms += kid.totals.tool_ms;
        totals.output_tokens += kid.totals.output_tokens;
        totals.input_tokens += kid.totals.input_tokens;
        match &kid.kind {
            NodeKind::Model(model) => {
                let start = kid.start.unwrap_or(end);
                totals.model_ms += (kid.end.unwrap_or(end) - start).max(0);
                totals.output_tokens += model.step.output_tokens();
                totals.input_tokens += model.step.input_tokens();
            }
            NodeKind::Call { call, agent: None } => {
                if let Some(call_end) = call.end() {
                    totals.tool_ms += call_end - call.start();
                }
            }
            _ => {}
        }
        errors += kid.errors + usize::from(kid.status == Some(Status::Error));
        kid_running |= kid.running;
    }
    let node = &mut nodes[id.0];
    node.children = kids;
    node.totals = totals;
    node.errors = errors;
    node.running = node.end.is_none() || kid_running;
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::*;

    /// Builders shaped like the hook's records.
    pub(crate) fn prompt(ms: i64, text: &str) -> Value {
        json!({"event": "UserPromptSubmit", "epoch_ms": ms, "agent_id": "main", "agent_type": "main", "prompt": text})
    }
    pub(crate) fn stop(ms: i64) -> Value {
        json!({"event": "Stop", "epoch_ms": ms, "agent_id": "main", "agent_type": "main",
               "extra": {"last_assistant_message": "done"}})
    }
    pub(crate) fn pre(ms: i64, id: &str, tool: &str, input: &Value, agent: &str) -> Value {
        let agent_type = if agent == "main" {
            json!("main")
        } else {
            Value::Null
        };
        json!({"event": "PreToolUse", "epoch_ms": ms, "tool_use_id": id, "tool_name": tool,
               "input": input, "agent_id": agent, "agent_type": agent_type, "status": "started"})
    }
    /// `record` with `fields` added, such as a producer's tool fields.
    pub(crate) fn with(mut record: Value, fields: &Value) -> Value {
        if let (Some(record), Some(fields)) = (record.as_object_mut(), fields.as_object()) {
            record.extend(fields.clone());
        }
        record
    }
    pub(crate) fn bash(ms: i64, id: &str) -> Value {
        pre(ms, id, "Bash", &json!({"command": "ls"}), "main")
    }
    pub(crate) fn post(ms: i64, id: &str, extra: &Value) -> Value {
        let mut record = json!({"event": "PostToolUse", "epoch_ms": ms, "tool_use_id": id,
                                "status": "ok", "duration_ms": 5});
        if let (Some(record), Some(extra)) = (record.as_object_mut(), extra.as_object()) {
            record.extend(extra.clone());
        }
        record
    }
    pub(crate) fn step(key: &str, first: i64, last: i64, extra: &Value) -> Value {
        let mut record = json!({"key": key, "agent_id": "main", "first_ms": first, "last_ms": last,
                                "usage": {"output_tokens": 10, "input_tokens": 100}});
        if let (Some(record), Some(extra)) = (record.as_object_mut(), extra.as_object()) {
            record.extend(extra.clone());
        }
        record
    }

    pub(crate) fn data(events: &[Value], steps: &[Value]) -> SessionData {
        let mut data = SessionData::default();
        for (row, record) in (1..).zip(events) {
            data.add_event(Event::parse(row, &record.to_string()).unwrap());
        }
        for record in steps {
            data.add_step(serde_json::from_value(record.clone()).unwrap());
        }
        data
    }

    pub(crate) const LIVE: Moment = Moment {
        at: None,
        live: true,
        clock: 10_000,
    };
    pub(crate) const DONE: Moment = Moment {
        at: None,
        live: false,
        clock: 10_000,
    };

    fn node<'a>(model: &'a SessionModel, key: &str) -> &'a Node {
        model.node(model.find(&Key::from(key)).unwrap())
    }

    fn keys(model: &SessionModel, ids: &[NodeId]) -> Vec<String> {
        ids.iter()
            .map(|&id| model.node(id).key().to_string())
            .collect()
    }

    #[test]
    fn each_prompt_starts_a_turn_stop_closes_it_and_calls_go_inside() {
        let events = [
            prompt(1, "do it"),
            bash(2, "t1"),
            post(3, "t1", &json!({})),
            stop(4),
            prompt(5, "do it"),
        ];
        let model = build_session(&data(&events, &[]), LIVE);
        let turns = model.turns();
        assert_eq!(turns.len(), 2);
        assert_eq!(keys(&model, model.node(turns[0]).children()), ["t1"]);
        assert_eq!(model.node(turns[0]).end(), Some(4));
        assert_eq!(model.node(turns[0]).status(), Some(Status::Ok));
        assert_eq!(
            model.node(turns[1]).end(),
            None,
            "the last turn is still going while live"
        );
    }

    #[test]
    fn a_node_is_named_by_its_first_event_and_a_model_step_by_none() {
        let events = [prompt(1, "do it"), bash(2, "t1"), post(3, "t1", &json!({}))];
        let steps = [step("m1", 2, 2, &json!({}))];
        let model = build_session(&data(&events, &steps), LIVE);
        assert_eq!(model.node(model.turns()[0]).row_id(), Some(1));
        assert_eq!(node(&model, "t1").row_id(), Some(2));
        assert_eq!(node(&model, "m1").row_id(), None);
    }

    #[test]
    fn a_call_without_post_is_unfinished_in_a_closed_turn_and_running_in_the_open_one() {
        let events = [
            prompt(1, "do it"),
            bash(2, "lost"),
            prompt(5, "do it"),
            bash(6, "now"),
        ];
        let model = build_session(&data(&events, &[]), LIVE);
        assert_eq!(node(&model, "lost").status(), Some(Status::Unfinished));
        assert_eq!(
            node(&model, "lost").end(),
            Some(5),
            "it ends when the next prompt arrived"
        );
        assert_eq!(node(&model, "now").status(), Some(Status::Started));
    }

    #[test]
    fn when_not_live_the_last_turn_without_stop_ends_at_its_last_child() {
        let events = [prompt(1, "do it"), bash(2, "t1"), post(8, "t1", &json!({}))];
        let model = build_session(&data(&events, &[]), DONE);
        assert_eq!(model.node(model.turns()[0]).end(), Some(8));
    }

    #[test]
    fn a_subagents_calls_hang_under_the_agent_call_that_started_it() {
        let events = [
            prompt(1, "do it"),
            pre(
                2,
                "agent-call",
                "Agent",
                &json!({"description": "look around", "subagent_type": "Explore"}),
                "main",
            ),
            json!({"event": "SubagentStart", "epoch_ms": 3, "agent_id": "a1", "agent_type": "Explore"}),
            pre(4, "sub-read", "Read", &json!({"file_path": "/x"}), "a1"),
            post(5, "sub-read", &json!({})),
            json!({"event": "SubagentStop", "epoch_ms": 6, "agent_id": "a1", "agent_type": "Explore"}),
            post(7, "agent-call", &json!({"spawned_agent": "a1"})),
            stop(8),
        ];
        let model = build_session(&data(&events, &[]), DONE);
        let call = node(&model, "agent-call");
        assert_eq!(call.agent().unwrap().agent_type(), Some("Explore"));
        assert_eq!(keys(&model, call.children()), ["sub-read"]);
        assert_eq!(node(&model, "sub-read").depth(), 2);
        assert_eq!(
            keys(&model, model.node(model.turns()[0]).children()),
            ["agent-call"]
        );
    }

    #[test]
    fn an_agent_call_without_spawned_agent_does_not_adopt_a_subagent() {
        let events = [
            prompt(1, "do it"),
            pre(2, "agent-call", "Agent", &json!({}), "main"),
            json!({"event": "SubagentStart", "epoch_ms": 3, "agent_id": "a1", "agent_type": "Explore"}),
            pre(4, "sub-read", "Read", &json!({}), "a1"),
            post(5, "sub-read", &json!({})),
            json!({"event": "SubagentStop", "epoch_ms": 6, "agent_id": "a1", "agent_type": "Explore"}),
            post(7, "agent-call", &json!({"response": {"agentId": "a1"}})),
            stop(8),
        ];
        let model = build_session(&data(&events, &[]), DONE);
        assert!(
            node(&model, "agent-call").children().is_empty(),
            "core reads `spawned_agent` only, never the tool's response"
        );
        assert_eq!(
            keys(&model, model.node(model.turns()[0]).children()),
            ["agent-call", "agent:a1"]
        );
    }

    #[test]
    fn an_untyped_subagent_with_no_calls_is_a_progress_note() {
        let events = [
            prompt(1, "do it"),
            json!({"event": "SubagentStop", "epoch_ms": 2, "agent_id": "n1", "agent_type": null,
                   "extra": {"last_assistant_message": "note"}}),
        ];
        let model = build_session(&data(&events, &[]), DONE);
        assert!(matches!(node(&model, "agent:n1").kind(), NodeKind::Note(_)));
    }

    #[test]
    fn a_model_step_starts_when_its_agent_was_last_waiting_on_a_tool_result() {
        let events = [
            prompt(1000, "do it"),
            bash(1100, "t1"),
            post(1500, "t1", &json!({})),
            stop(3000),
        ];
        let steps = [
            step(
                "model:main:m1",
                1050,
                1090,
                &json!({"tool_use_ids": ["t1"]}),
            ),
            step("model:main:m2", 2000, 2900, &json!({})),
        ];
        let model = build_session(&data(&events, &steps), DONE);
        assert_eq!(
            node(&model, "model:main:m1").start(),
            Some(1000),
            "first step waited from the prompt"
        );
        assert_eq!(
            node(&model, "model:main:m2").start(),
            Some(1500),
            "second step waited from the tool result"
        );
        let totals = model.node(model.turns()[0]).totals();
        assert_eq!(totals.model_ms, 90 + 1400);
        assert_eq!(totals.tool_ms, 400);
        assert_eq!(totals.output_tokens, 20);
    }

    #[test]
    fn replaying_an_earlier_moment_shows_the_session_as_it_was_then() {
        let events = [
            prompt(1000, "do it"),
            bash(1100, "t1"),
            post(1500, "t1", &json!({})),
            stop(3000),
        ];
        let steps = [step("model:main:m1", 1200, 1800, &json!({}))];
        let moment = Moment {
            at: Some(1300),
            live: false,
            clock: 10_000,
        };
        let model = build_session(&data(&events, &steps), moment);
        assert_eq!(
            node(&model, "t1").status(),
            Some(Status::Started),
            "its Post had not happened yet"
        );
        assert_eq!(
            node(&model, "model:main:m1").status(),
            Some(Status::Started),
            "the step was still being written"
        );
        assert_eq!(model.node(model.turns()[0]).end(), None);
    }

    #[test]
    fn open_end_follows_the_clock_while_live_and_recent_else_the_last_event() {
        let bounds = Some(Bounds { t0: 0, t1: 1000 });
        let at = |at, live, clock| open_end(bounds, Moment { at, live, clock });
        assert_eq!(at(Some(500), true, 2000), 500);
        assert_eq!(at(None, true, 2000), 2000);
        assert_eq!(at(None, true, 1000 + STALE_MS + 1), 1000);
        assert_eq!(at(None, false, 2000), 1000);
    }

    #[test]
    fn stats_agent_graph_and_state_at_agree_with_the_tree() {
        let events = [
            prompt(1, "do it"),
            with(
                pre(2, "w", "Write", &json!({}), "main"),
                &json!({"files": [{"path": "/a.rs", "op": "write"}]}),
            ),
            post(3, "w", &json!({})),
            with(
                pre(4, "r", "Read", &json!({}), "main"),
                &json!({"files": [{"path": "/b.rs", "op": "read"}]}),
            ),
            post(5, "r", &json!({"status": "error"})),
            with(
                pre(6, "todo", "TodoWrite", &json!({}), "main"),
                &json!({"todos": [{"content": "x", "status": "pending"}]}),
            ),
            post(7, "todo", &json!({})),
            stop(8),
        ];
        let model = build_session(&data(&events, &[]), DONE);
        let stats = model.stats();
        assert_eq!((stats.calls, stats.errors, stats.turns), (3, 1, 1));

        let graph = model.agent_graph(|_| String::new());
        let tools: Vec<&str> = graph
            .main()
            .tools()
            .iter()
            .map(|(t, _)| t.as_str())
            .collect();
        assert_eq!(tools, ["Write", "Read", "TodoWrite"]);
        assert_eq!(graph.main().errors(), 1);

        let early = model.state_at(4);
        let paths_of =
            |files: &[FileTouch]| files.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths_of(&early.changed), ["/a.rs"]);
        assert_eq!(paths_of(&early.read), ["/b.rs"]);
        assert_eq!(early.todos, None, "the todo list came later");
        assert_eq!(model.state_at(8).todos.map(|todos| todos.len()), Some(1));
    }

    #[test]
    fn moments_and_raw_data_cover_every_event() {
        let events = [prompt(1, "do it"), bash(2, "t1"), post(3, "t1", &json!({}))];
        let steps = [step("model:main:m1", 2, 4, &json!({}))];
        let data = data(&events, &steps);
        assert_eq!(moments(data.events(), data.steps()), [1, 2, 3, 4]);
        let model = build_session(&data, DONE);
        assert_eq!(node(&model, "t1").raw_data().len(), 2);
    }
}
