//! What the viewer shows and where the user is: view, selection, expanded rows, filter and
//! replay cursor. No GPUI types, so keyboard and replay behavior are unit-tested here.

use std::collections::{HashMap, HashSet};

use ichnos_model::call::{Call, Key};
use ichnos_model::filter::Filter;
use ichnos_model::labels::Summaries;
use ichnos_model::model::{Bounds, NodeId, NodeKind, SessionModel};

pub use ichnos_core::instance::View;

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

    /// Start over for another session; the view and filter stay.
    pub fn reset_for_session(&mut self) {
        *self = Self {
            view: self.view,
            query: std::mem::take(&mut self.query),
            show_model: self.show_model,
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

/// The right-hand panel's default width, in pixels.
pub const PANEL_DEFAULT: f32 = 380.;

/// The right-hand panel's narrowest and widest, in pixels.
pub const PANEL_RANGE: (f32, f32) = (260., 720.);

/// The width, in pixels, the main pane keeps however wide the panel is dragged.
pub const MAIN_MIN: f32 = 320.;

/// The width of the right-hand panel, and the resize drag in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelWidth {
    width: f32,
    /// Where the pointer and the width were when the drag started.
    drag: Option<(f32, f32)>,
}

impl Default for PanelWidth {
    fn default() -> Self {
        Self {
            width: PANEL_DEFAULT,
            drag: None,
        }
    }
}

impl PanelWidth {
    /// The width, in pixels.
    #[must_use]
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Whether a drag is in progress.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Back to the default width.
    pub fn reset(&mut self) {
        self.width = PANEL_DEFAULT;
    }

    /// The pointer went down at `x`.
    pub fn start(&mut self, x: f32) {
        self.drag = Some((x, self.width));
    }

    /// The pointer moved to `x` in a window `window` pixels wide; `true` when the width changed.
    /// Dragging left widens the panel.
    pub fn drag_to(&mut self, x: f32, window: f32) -> bool {
        let Some((from, width)) = self.drag else {
            return false;
        };
        let most = (window - MAIN_MIN).clamp(PANEL_RANGE.0, PANEL_RANGE.1);
        let next = (width + from - x).clamp(PANEL_RANGE.0, most);
        let moved = (next - self.width).abs() > f32::EPSILON;
        self.width = next;
        moved
    }

    /// The pointer went up.
    pub fn end(&mut self) {
        self.drag = None;
    }
}

/// Pointer travel, in pixels, beyond which a press counts as a drag rather than a click.
const DRAG_THRESHOLD: f32 = 3.;

/// The graph's smallest and largest zoom.
pub const ZOOM_RANGE: (f32, f32) = (0.25, 3.);

/// How the graph is moved from its centered position and zoomed, and the drag in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pan {
    /// In screen pixels.
    offset: (f32, f32),
    zoom: f32,
    /// Where the pointer and the offset were when the drag started.
    drag: Option<((f32, f32), (f32, f32))>,
    /// Whether the last press moved the graph, so its release isn't a click on a card.
    dragged: bool,
}

impl Default for Pan {
    fn default() -> Self {
        Self {
            offset: (0., 0.),
            zoom: 1.,
            drag: None,
            dragged: false,
        }
    }
}

impl Pan {
    /// The zoom: 1 is actual size.
    #[must_use]
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Zoom by `factor` (above 1 zooms in) within [`ZOOM_RANGE`], keeping what's at the center
    /// of the pane in place.
    pub fn zoom_by(&mut self, factor: f32) {
        let zoom = (self.zoom * factor).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        let ratio = zoom / self.zoom;
        self.offset = (self.offset.0 * ratio, self.offset.1 * ratio);
        self.zoom = zoom;
    }

    /// Back to actual size, keeping the center in place.
    pub fn reset_zoom(&mut self) {
        self.zoom_by(1. / self.zoom);
    }

    /// The offset from the centered position.
    #[must_use]
    pub fn offset(&self) -> (f32, f32) {
        self.offset
    }

    /// Whether a drag is in progress.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Whether the last press moved the graph rather than clicked.
    #[must_use]
    pub fn was_drag(&self) -> bool {
        self.dragged
    }

    /// The pointer went down at `pointer`.
    pub fn start(&mut self, pointer: (f32, f32)) {
        self.drag = Some((pointer, self.offset));
        self.dragged = false;
    }

    /// The pointer moved to `pointer`; `true` when the graph moved.
    pub fn drag_to(&mut self, pointer: (f32, f32)) -> bool {
        let Some((from, offset)) = self.drag else {
            return false;
        };
        let (dx, dy) = (pointer.0 - from.0, pointer.1 - from.1);
        if !self.dragged && dx.hypot(dy) < DRAG_THRESHOLD {
            return false;
        }
        self.dragged = true;
        self.offset = (offset.0 + dx, offset.1 + dy);
        true
    }

    /// The pointer went up.
    pub fn end(&mut self) {
        self.drag = None;
    }

    /// Move the graph by a scroll of `delta`.
    pub fn scroll(&mut self, delta: (f32, f32)) {
        self.offset = (self.offset.0 + delta.0, self.offset.1 + delta.1);
    }

    /// Back to the centered position.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use ichnos_core::event::Event;
    use ichnos_model::model::{Moment, build_session};
    use ichnos_model::session_data::SessionData;
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn dragging_the_handle_left_widens_the_panel() {
        let mut panel = PanelWidth::default();
        panel.start(1000.);
        assert!(panel.drag_to(940., 1600.));
        assert_eq!(panel.width(), PANEL_DEFAULT + 60.);
        panel.end();
        assert!(!panel.drag_to(900., 1600.));
        assert!(!panel.is_dragging());
    }

    #[test]
    fn the_panel_width_stays_within_bounds() {
        let mut panel = PanelWidth::default();
        panel.start(1000.);
        panel.drag_to(-5000., 1600.);
        assert_eq!(panel.width(), PANEL_RANGE.1);
        panel.drag_to(5000., 1600.);
        assert_eq!(panel.width(), PANEL_RANGE.0);
        panel.drag_to(-5000., 800.);
        assert_eq!(panel.width(), 800. - MAIN_MIN);
        panel.reset();
        assert_eq!(panel.width(), PANEL_DEFAULT);
    }

    const DONE: Moment = Moment {
        at: None,
        live: false,
        clock: 10_000,
    };

    fn model(events: &[Value]) -> SessionModel {
        let mut data = SessionData::default();
        for (row, record) in (1..).zip(events) {
            data.add_event(Event::parse(row, &record.to_string()).unwrap());
        }
        build_session(&data, DONE)
    }

    fn two_turns() -> SessionModel {
        model(&[
            json!({"event": "UserPromptSubmit", "epoch_ms": 1, "prompt": "first"}),
            json!({"event": "PreToolUse", "epoch_ms": 2, "tool_use_id": "a", "tool_name": "Bash"}),
            json!({"event": "PostToolUseFailure", "epoch_ms": 3, "tool_use_id": "a", "status": "error"}),
            json!({"event": "Stop", "epoch_ms": 4}),
            json!({"event": "UserPromptSubmit", "epoch_ms": 5, "prompt": "second"}),
            json!({"event": "PreToolUse", "epoch_ms": 6, "tool_use_id": "b", "tool_name": "Read"}),
            json!({"event": "PostToolUse", "epoch_ms": 7, "tool_use_id": "b", "status": "ok"}),
            json!({"event": "PreToolUse", "epoch_ms": 8, "tool_use_id": "c", "tool_name": "Bash"}),
            json!({"event": "PostToolUseFailure", "epoch_ms": 9, "tool_use_id": "c", "status": "error"}),
            json!({"event": "Stop", "epoch_ms": 10}),
        ])
    }

    fn keys(model: &SessionModel, rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| model.node(row.id).key().to_string())
            .collect()
    }

    #[test]
    fn only_the_last_turn_starts_open() {
        let model = two_turns();
        let state = ViewState::default();
        let rows = state.tree_rows(&model, &Summaries::new());
        assert_eq!(
            keys(&model, &rows),
            [
                "turn:UserPromptSubmit:1:main",
                "turn:UserPromptSubmit:5:main",
                "b",
                "c"
            ]
        );
    }

    #[test]
    fn toggling_a_turn_opens_and_closes_it() {
        let model = two_turns();
        let mut state = ViewState::default();
        let first = model.turns()[0];
        state.toggle(&model, first);
        assert_eq!(state.tree_rows(&model, &Summaries::new()).len(), 5);
        state.toggle(&model, first);
        assert_eq!(state.tree_rows(&model, &Summaries::new()).len(), 4);
    }

    #[test]
    fn filtering_opens_every_turn_and_keeps_matches() {
        let model = two_turns();
        let mut state = ViewState::default();
        state.set_query("is:error");
        let rows = state.tree_rows(&model, &Summaries::new());
        assert_eq!(
            keys(&model, &rows),
            [
                "turn:UserPromptSubmit:1:main",
                "a",
                "turn:UserPromptSubmit:5:main",
                "c"
            ]
        );
    }

    #[test]
    fn j_and_k_move_through_rows_and_stop_at_the_ends() {
        let order: Vec<Key> = ["x", "y", "z"].map(Key::from).to_vec();
        let mut state = ViewState::default();
        state.move_selection(&order, 1);
        assert_eq!(state.selected(), Some(&Key::from("x")));
        state.move_selection(&order, 1);
        state.move_selection(&order, 1);
        state.move_selection(&order, 1);
        assert_eq!(state.selected(), Some(&Key::from("z")));
        state.move_selection(&order, -5);
        assert_eq!(state.selected(), Some(&Key::from("x")));
        assert!(!state.tail(), "moving by hand stops auto-scroll");
    }

    #[test]
    fn next_error_cycles_through_failed_calls_and_opens_their_turn() {
        let model = two_turns();
        let mut state = ViewState::new(View::Graph);
        assert_eq!(state.next_error(&model), Some(Key::from("a")));
        assert_eq!(state.view(), View::Tree, "errors are shown in the tree");
        let rows = state.tree_rows(&model, &Summaries::new());
        assert!(keys(&model, &rows).contains(&"a".to_owned()));
        assert_eq!(state.next_error(&model), Some(Key::from("c")));
        assert_eq!(state.next_error(&model), Some(Key::from("a")));
        assert_eq!(state.take_pending_reveal(), Some(Key::from("a")));
    }

    #[test]
    fn replay_steps_through_moments_and_runs_off_the_end_into_live() {
        let mut state = ViewState::default();
        let bounds = Some(Bounds { t0: 1, t1: 3 });
        state.toggle_play(bounds);
        assert_eq!((state.replay_at(), state.playing()), (Some(1), true));
        state.step(&[1, 2, 3], true);
        assert_eq!(state.replay_at(), Some(2));
        state.step(&[1, 2, 3], true);
        state.step(&[1, 2, 3], true);
        assert_eq!((state.replay_at(), state.playing()), (None, false));
        state.step(&[1, 2, 3], false);
        assert_eq!(state.replay_at(), Some(2));
    }

    #[test]
    fn seeking_past_the_end_goes_live() {
        let mut state = ViewState::default();
        let bounds = Some(Bounds { t0: 10, t1: 20 });
        state.seek(bounds, 5);
        assert_eq!(state.replay_at(), Some(10));
        state.seek(bounds, 20);
        assert_eq!(state.replay_at(), None);
    }

    #[test]
    fn the_table_lists_subagent_events_without_a_tree_node() {
        let model = model(&[
            json!({"event": "UserPromptSubmit", "epoch_ms": 1, "prompt": "go"}),
            json!({"event": "SubagentStart", "epoch_ms": 2, "agent_id": "a1", "agent_type": "Explore"}),
        ]);
        let rows = ViewState::default().table_rows(&model, &Summaries::new());
        assert_eq!(rows.len(), 2);
        assert!(rows[1].node.is_none());
    }

    #[test]
    fn dragging_moves_the_graph_and_a_short_press_is_still_a_click() {
        let mut pan = Pan::default();
        pan.start((100., 100.));
        assert!(!pan.drag_to((101., 101.)), "a jitter is not a drag");
        pan.end();
        assert!(!pan.was_drag());

        pan.start((100., 100.));
        assert!(pan.drag_to((130., 90.)));
        pan.end();
        assert!(pan.was_drag());
        assert_eq!(pan.offset(), (30., -10.));

        pan.start((0., 0.));
        pan.drag_to((10., 10.));
        assert_eq!(
            pan.offset(),
            (40., 0.),
            "a second drag continues from the first"
        );
        pan.end();
        pan.scroll((-40., 5.));
        assert_eq!(pan.offset(), (0., 5.));
        pan.reset();
        assert_eq!(pan, Pan::default());
    }

    #[test]
    fn zooming_scales_the_offset_and_stays_in_range() {
        let mut pan = Pan::default();
        pan.scroll((10., -20.));
        pan.zoom_by(2.);
        assert_eq!((pan.zoom(), pan.offset()), (2., (20., -40.)));
        pan.zoom_by(10.);
        assert_eq!(pan.zoom(), ZOOM_RANGE.1);
        pan.zoom_by(0.001);
        assert_eq!(pan.zoom(), ZOOM_RANGE.0);
        pan.reset_zoom();
        assert!((pan.zoom() - 1.).abs() < f32::EPSILON);
        assert!((pan.offset().0 - 10.).abs() < 1e-4 && (pan.offset().1 + 20.).abs() < 1e-4);
    }
}
