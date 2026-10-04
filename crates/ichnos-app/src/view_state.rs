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

/// The right-hand panel's default width, in pixels.
pub const PANEL_DEFAULT: f32 = 380.;

/// The right-hand panel's narrowest and widest, in pixels.
pub const PANEL_RANGE: (f32, f32) = (260., 720.);

/// The width, in pixels, the main pane keeps however wide the panel is dragged.
pub const MAIN_MIN: f32 = 320.;

/// The width, in pixels, of the divider between the main pane and the panel.
pub const HANDLE_WIDTH: f32 = 5.;

/// The main pane width, in pixels, below which tree rows drop their timeline bar so the label
/// keeps its room.
pub const BARS_MIN_MAIN: f32 = 640.;

/// The width of the right-hand panel, whether it shows, and the resize drag in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelWidth {
    width: f32,
    open: bool,
    /// Where the pointer and the width were when the drag started.
    drag: Option<(f32, f32)>,
}

impl Default for PanelWidth {
    fn default() -> Self {
        Self {
            width: PANEL_DEFAULT,
            open: true,
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

    /// Whether the panel shows.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Show the panel if hidden, hide it if shown; its width is kept for next time.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.drag = None;
    }

    /// The main pane's width in a window `window` pixels wide.
    #[must_use]
    pub fn main_width(&self, window: f32) -> f32 {
        if self.open {
            window - self.width - HANDLE_WIDTH
        } else {
            window
        }
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

    #[test]
    fn a_hidden_panel_gives_the_main_pane_the_whole_window() {
        let mut panel = PanelWidth::default();
        assert_eq!(
            panel.main_width(1000.),
            1000. - PANEL_DEFAULT - HANDLE_WIDTH
        );
        panel.toggle();
        assert!(!panel.is_open());
        assert_eq!(panel.main_width(1000.), 1000.);
        panel.toggle();
        assert_eq!(panel.width(), PANEL_DEFAULT);
    }

    #[test]
    fn switching_sessions_keeps_the_panel() {
        let mut state = ViewState::default();
        state.panel_mut().toggle();
        state.reset_for_session();
        assert!(!state.panel().is_open());
    }

    #[test]
    fn an_agent_reference_names_the_session_and_the_event() {
        let session: SessionId = "afde7f06-8de7-4806-9714-a72e071cedd9".parse().unwrap();
        assert_eq!(
            agent_reference(&session, 68),
            "/ichnos afde7f06-8de7-4806-9714-a72e071cedd9#68"
        );
    }

    #[test]
    fn the_nearest_tick_within_reach_is_hovered() {
        let ticks = [0.1, 0.5, 0.52, 0.9];
        assert_eq!(nearest_tick(&ticks, 0.515, 0.02), Some(2));
        assert_eq!(nearest_tick(&ticks, 0.11, 0.02), Some(0));
        assert_eq!(nearest_tick(&ticks, 0.7, 0.02), None);
        assert_eq!(nearest_tick(&[], 0.5, 1.), None);
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

    #[test]
    fn related_matches_skip_listed_and_deleted_sessions() {
        let id = |text: &str| -> SessionId { text.parse().unwrap() };
        let found = |session: &str| SessionMatch {
            session: id(session),
            event_id: 1,
            kind: ichnos_search::DocKind::Summary,
            text: session.to_owned(),
            distance: 0.1,
        };
        let (a, b, c) = (id("a"), id("b"), id("c"));
        let related = [found("a"), found("gone"), found("c"), found("b")];
        let kept = related_matches(&related, &[&a], &[&a, &b, &c]);
        let sessions: Vec<&str> = kept.iter().map(|found| found.session.as_str()).collect();
        assert_eq!(sessions, ["c", "b"]);
    }

    #[test]
    fn scrubbing_is_held_between_press_and_release() {
        let mut state = ViewState::default();
        assert!(!state.scrubbing());
        state.set_scrubbing(true);
        assert!(state.scrubbing());
        state.set_scrubbing(false);
        assert!(!state.scrubbing());
    }
}
