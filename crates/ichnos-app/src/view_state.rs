//! What the viewer shows and where the user is: view, selection, expanded rows, filter and
//! replay cursor. No GPUI types, so keyboard and replay behavior are unit-tested here.

use std::collections::{HashMap, HashSet};

use ichnos_core::store::SessionId;
use ichnos_model::call::{Call, Key};
use ichnos_model::filter::Filter;
use ichnos_model::labels::Summaries;
use ichnos_model::model::{Bounds, NodeId, NodeKind, SessionModel};
use ichnos_search::SessionMatch;

pub use ichnos_core::instance::View;
pub use pan::Pan;
pub use panel::{BARS_MIN_MAIN, HANDLE_WIDTH, PanelWidth};

mod pan;
mod panel;
#[cfg(test)]
mod tests;

/// The inspector's tab.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InspectorTab {
    /// The selected row's input and output.
    #[default]
    Call,
    /// What was running at the inspected moment.
    State,
    /// Files changed and read.
    Files,
    /// Token use.
    Tokens,
}

/// One row of the tree: a node and the turn it's in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRow {
    /// The node shown.
    pub id: NodeId,
    /// Its turn.
    pub turn: NodeId,
}

/// One row of the table: an entry, and its tree node when it has one (subagent start and stop
/// events have none).
#[derive(Debug, Clone, PartialEq)]
pub struct TableRow {
    /// The entry.
    pub call: Call,
    /// Its tree node.
    pub node: Option<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Scrub {
    #[default]
    Idle,
    Held,
}

/// Interaction state of one viewer window.
#[derive(Debug, Clone)]
pub struct ViewState {
    view: View,
    tab: InspectorTab,
    query: String,
    show_model: bool,
    /// Keep the newest row in sight as data arrives.
    tail: bool,
    replay_at: Option<i64>,
    playing: bool,
    /// Whether a press on the replay strip is held, so moving the pointer scrubs.
    scrub: Scrub,
    /// Node key -> expanded, only for nodes the user toggled.
    open: HashMap<Key, bool>,
    selected: Option<Key>,
    /// The agent selected in the graph.
    agent: String,
    /// Index into the failed calls of the last "next error" jump.
    error_jump: Option<usize>,
    /// A row to scroll into view on the next render.
    pending_reveal: Option<Key>,
    /// How the graph is moved from its centered position.
    graph_pan: Pan,
    /// The width of the right-hand panel.
    panel: PanelWidth,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            view: View::default(),
            tab: InspectorTab::default(),
            query: String::new(),
            show_model: true,
            tail: true,
            replay_at: None,
            playing: false,
            scrub: Scrub::Idle,
            open: HashMap::new(),
            selected: None,
            agent: ichnos_core::event::MAIN_AGENT.to_owned(),
            error_jump: None,
            pending_reveal: None,
            graph_pan: Pan::default(),
            panel: PanelWidth::default(),
        }
    }
}

impl ViewState {
    /// A fresh state showing `view`.
    #[must_use]
    pub fn new(view: View) -> Self {
        Self {
            view,
            ..Self::default()
        }
    }

    /// Start over for another session; the view, filter and panel stay.
    pub fn reset_for_session(&mut self) {
        *self = Self {
            view: self.view,
            query: std::mem::take(&mut self.query),
            show_model: self.show_model,
            panel: PanelWidth {
                drag: None,
                ..self.panel
            },
            ..Self::default()
        };
    }

    /// The main area's view.
    #[must_use]
    pub fn view(&self) -> View {
        self.view
    }

    /// Show `view`.
    pub fn set_view(&mut self, view: View) {
        self.view = view;
    }

    /// The inspector's tab.
    #[must_use]
    pub fn tab(&self) -> InspectorTab {
        self.tab
    }

    /// Show inspector tab `tab`.
    pub fn set_tab(&mut self, tab: InspectorTab) {
        self.tab = tab;
    }

    /// The filter box's text.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Change the filter box's text.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    /// The parsed filter.
    #[must_use]
    pub fn filter(&self) -> Filter {
        Filter::parse(&self.query)
    }

    /// Whether model steps are shown in the tree.
    #[must_use]
    pub fn show_model(&self) -> bool {
        self.show_model
    }

    /// Show or hide model steps.
    pub fn set_show_model(&mut self, show: bool) {
        self.show_model = show;
    }

    /// Whether the newest row is kept in sight.
    #[must_use]
    pub fn tail(&self) -> bool {
        self.tail
    }

    /// Keep the newest row in sight, or stop doing so.
    pub fn set_tail(&mut self, tail: bool) {
        self.tail = tail;
    }

    /// The replay cursor; `None` while live.
    #[must_use]
    pub fn replay_at(&self) -> Option<i64> {
        self.replay_at
    }

    /// Whether replay is playing.
    #[must_use]
    pub fn playing(&self) -> bool {
        self.playing
    }

    /// The selected row's key.
    #[must_use]
    pub fn selected(&self) -> Option<&Key> {
        self.selected.as_ref()
    }

    /// The agent selected in the graph.
    #[must_use]
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// Select agent `id` in the graph.
    pub fn select_agent(&mut self, id: &str) {
        id.clone_into(&mut self.agent);
    }

    /// How the graph is moved from its centered position.
    #[must_use]
    pub fn graph_pan(&self) -> &Pan {
        &self.graph_pan
    }

    /// Move the graph.
    pub fn graph_pan_mut(&mut self) -> &mut Pan {
        &mut self.graph_pan
    }

    /// The width of the right-hand panel.
    #[must_use]
    pub fn panel(&self) -> &PanelWidth {
        &self.panel
    }

    /// Resize the right-hand panel.
    pub fn panel_mut(&mut self) -> &mut PanelWidth {
        &mut self.panel
    }

    /// The row to scroll into view, taken once.
    pub fn take_pending_reveal(&mut self) -> Option<Key> {
        self.pending_reveal.take()
    }

    /// Select row `key`, showing its call; `scroll` brings it into view.
    pub fn select(&mut self, key: Key, scroll: bool) {
        if scroll {
            self.pending_reveal = Some(key.clone());
        }
        self.selected = Some(key);
        self.tab = InspectorTab::Call;
    }

    /// Whether node `id`'s children are shown.
    #[must_use]
    pub fn is_open(&self, model: &SessionModel, id: NodeId, filtering: bool) -> bool {
        let node = model.node(id);
        if node.children().is_empty() {
            return false;
        }
        if filtering {
            return true;
        }
        if let Some(&open) = self.open.get(node.key()) {
            return open;
        }
        if node.is_turn() {
            return model.turns().last() == Some(&id) || node.running();
        }
        true
    }

    /// Expand or collapse node `id`.
    pub fn toggle(&mut self, model: &SessionModel, id: NodeId) {
        let open = self.is_open(model, id, self.filter().is_active());
        self.open.insert(model.node(id).key().clone(), !open);
    }

    /// Expand or collapse every node with children.
    pub fn set_all_open(&mut self, model: &SessionModel, open: bool) {
        self.open = model
            .iter()
            .filter(|(_, node)| !node.children().is_empty())
            .map(|(_, node)| (node.key().clone(), open))
            .collect();
    }

    /// The tree's rows in order: what is expanded and passes the filter.
    #[must_use]
    pub fn tree_rows(&self, model: &SessionModel, summaries: &Summaries) -> Vec<TreeRow> {
        let filter = self.filter();
        let filtering = filter.is_active();
        let keep: Option<HashSet<Key>> = filtering.then(|| filter.visible_keys(model, summaries));
        let mut rows = Vec::new();
        for &turn in model.turns() {
            self.walk(model, turn, turn, keep.as_ref(), filtering, &mut rows);
        }
        rows
    }

    fn walk(
        &self,
        model: &SessionModel,
        id: NodeId,
        turn: NodeId,
        keep: Option<&HashSet<Key>>,
        filtering: bool,
        rows: &mut Vec<TreeRow>,
    ) {
        let node = model.node(id);
        if keep.is_some_and(|keep| !keep.contains(node.key())) {
            return;
        }
        if matches!(node.kind(), NodeKind::Model(_)) && !self.show_model {
            return;
        }
        rows.push(TreeRow { id, turn });
        if self.is_open(model, id, filtering) {
            for &kid in node.children() {
                self.walk(model, kid, turn, keep, filtering, rows);
            }
        }
    }

    /// Every entry for the table, subagent start and stop events included, filtered.
    #[must_use]
    pub fn table_rows(&self, model: &SessionModel, summaries: &Summaries) -> Vec<TableRow> {
        let filter = self.filter();
        model
            .calls()
            .iter()
            .map(|call| TableRow {
                call: call.clone(),
                node: model.find(call.key()),
            })
            .filter(|row| {
                // Entries without a tree node only match an empty filter.
                !filter.is_active()
                    || row
                        .node
                        .is_some_and(|id| filter.matches(model.node(id), summaries, model))
            })
            .collect()
    }

    /// Move the selection `delta` rows through `order`; with nothing selected, start at an end.
    pub fn move_selection(&mut self, order: &[Key], delta: isize) {
        if order.is_empty() {
            return;
        }
        let last = order.len() - 1;
        let at = self
            .selected
            .as_ref()
            .and_then(|key| order.iter().position(|k| k == key));
        let next = match at {
            Some(at) => at.saturating_add_signed(delta).min(last),
            None if delta > 0 => 0,
            None => last,
        };
        self.tail = false;
        self.select(order[next].clone(), true);
    }

    /// Expand every ancestor of `key`, select it and bring it into view in the tree.
    pub fn reveal(&mut self, model: &SessionModel, key: &Key) {
        if let Some(id) = model.find(key) {
            let mut parent = model.node(id).parent();
            while let Some(up) = parent {
                self.open.insert(model.node(up).key().clone(), true);
                parent = model.node(up).parent();
            }
            if matches!(model.node(id).kind(), NodeKind::Model(_)) {
                self.show_model = true;
            }
        }
        self.tail = false;
        if self.view == View::Graph {
            self.view = View::Tree;
        }
        self.select(key.clone(), true);
    }

    /// Reveal the next failed call, cycling; `None` when there are none.
    pub fn next_error(&mut self, model: &SessionModel) -> Option<Key> {
        let failed: Vec<Key> = failed_calls(model);
        if failed.is_empty() {
            return None;
        }
        let next = self.error_jump.map_or(0, |at| (at + 1) % failed.len());
        self.error_jump = Some(next);
        let key = failed[next].clone();
        self.reveal(model, &key);
        Some(key)
    }

    /// Move the replay cursor to `ms`; at or past the end is live.
    pub fn seek(&mut self, bounds: Option<Bounds>, ms: i64) {
        let Some(bounds) = bounds else {
            return;
        };
        self.replay_at = (ms < bounds.t1).then(|| ms.max(bounds.t0));
        if self.replay_at.is_some() {
            self.tail = false;
        }
    }

    /// Move the replay cursor to the previous or next moment in `moments`; past the last is live.
    pub fn step(&mut self, moments: &[i64], forward: bool) {
        let (Some(&first), Some(&last)) = (moments.first(), moments.last()) else {
            return;
        };
        let current = self.replay_at.unwrap_or(last);
        let next = if forward {
            moments.iter().copied().find(|&m| m > current)
        } else {
            Some(
                moments
                    .iter()
                    .rev()
                    .copied()
                    .find(|&m| m < current)
                    .unwrap_or(first),
            )
        };
        self.replay_at = next;
        if next.is_none() {
            self.playing = false;
        } else {
            self.tail = false;
        }
    }

    /// Start or pause replay; starting while live rewinds to the beginning.
    pub fn toggle_play(&mut self, bounds: Option<Bounds>) {
        if self.playing {
            self.playing = false;
            return;
        }
        let Some(bounds) = bounds else {
            return;
        };
        if self.replay_at.is_none() {
            self.replay_at = Some(bounds.t0);
            self.tail = false;
        }
        self.playing = true;
    }

    /// Whether a press on the replay strip is held.
    #[must_use]
    pub fn scrubbing(&self) -> bool {
        self.scrub == Scrub::Held
    }

    /// Note that a press on the replay strip started or ended.
    pub fn set_scrubbing(&mut self, held: bool) {
        self.scrub = if held { Scrub::Held } else { Scrub::Idle };
    }

    /// Stop playing.
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Stop replay and follow the session live.
    pub fn go_live(&mut self) {
        self.playing = false;
        self.replay_at = None;
    }
}

/// Every failed tool call, in tree order.
#[must_use]
pub fn failed_calls(model: &SessionModel) -> Vec<Key> {
    model
        .iter()
        .filter(|(_, node)| {
            matches!(node.kind(), NodeKind::Call { .. })
                && node.status() == Some(ichnos_core::event::Status::Error)
        })
        .map(|(_, node)| node.key().clone())
        .collect()
}

/// Event `row_id` of `session` as a prompt for an agent: `/ichnos <session>#<row_id>`, which
/// runs the `ichnos` skill on that event.
#[must_use]
pub fn agent_reference(session: &SessionId, row_id: i64) -> String {
    format!("/ichnos {}#{row_id}", session.as_str())
}

/// The tick nearest to `at` within `tolerance`, as an index into `ticks`; all three are
/// fractions of the replay strip's width.
#[must_use]
pub fn nearest_tick(ticks: &[f32], at: f32, tolerance: f32) -> Option<usize> {
    ticks
        .iter()
        .enumerate()
        .map(|(ix, &tick)| (ix, (tick - at).abs()))
        .filter(|&(_, distance)| distance <= tolerance)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(ix, _)| ix)
}

/// The meaning matches a switcher lists after its word matches: those whose session is in
/// `known` (its database still exists) and not already in `listed`, in their order.
pub fn related_matches(
    related: &[SessionMatch],
    listed: &[&SessionId],
    known: &[&SessionId],
) -> Vec<SessionMatch> {
    related
        .iter()
        .filter(|found| known.contains(&&found.session) && !listed.contains(&&found.session))
        .cloned()
        .collect()
}
