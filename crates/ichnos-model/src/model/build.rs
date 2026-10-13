//! Building a [`SessionModel`] from a session's events and steps, one phase per method.

use std::collections::HashMap;
use std::sync::Arc;

use super::{
    Agent, ModelStep, Moment, Node, NodeId, NodeKind, SessionModel, Totals, open_end,
    session_bounds,
};
use crate::call::{Call, Key, pair_calls};
use crate::session_data::SessionData;
use crate::step::Step;
use ichnos_core::event::{Event, MAIN_AGENT, Status};

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
