//! What the views count and draw: a session's headline numbers, the agent graph, the files it
//! touched and its state at one moment.

use std::sync::Arc;

use super::{Node, NodeId, NodeKind, SessionModel};
use crate::call::Key;
use crate::step::Step;
use ichnos_core::event::{Event, FileOp, MAIN_AGENT, Status, Todo};

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
    pub(super) boxes: Vec<AgentBox>,
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

    pub(super) fn position(&self, id: &str) -> Option<usize> {
        self.boxes.iter().position(|entry| entry.id == id)
    }

    pub(super) fn entry(&mut self, id: &str) -> &mut AgentBox {
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

impl SessionModel {
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
