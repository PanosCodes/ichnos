//! The viewer window's root: follows a session live, rebuilds its model and lays out the toolbar,
//! replay strip, main view and inspector.

mod agent_inspector;
mod ask_panel;
mod copy;
mod graph;
mod inspector;
mod keys;
mod polling;
mod replay;
mod switcher;
mod table;
#[cfg(test)]
mod tests;
mod toolbar;
mod tree;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, Render, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle, Window,
    div, px,
};
use ichnos_core::catalog::{Catalog, SessionEntry};
use ichnos_core::instance::Listener;
use ichnos_core::one_line;
use ichnos_core::store::SessionId;
use ichnos_model::call::Key;
use ichnos_model::labels::Summaries;
use ichnos_model::model::{Bounds, Moment, SessionModel, build_session, moments, session_bounds};
use ichnos_model::session_data::SessionData;
use ichnos_search::Search;

use self::ask_panel::AskPanel;
use self::keys::{
    FocusFilter, NextError, OpenSettings, ShowGraph, ShowTable, ShowTree, StepBack, StepForward,
    ToggleHelp, TogglePanel, TogglePlay, ToggleSwitcher, ZoomIn, ZoomOut, ZoomReset,
};
pub use self::keys::{bind_keys, set_menus};
use self::switcher::{Switcher, SwitcherEvent};
use crate::live::{Connection, LiveSession};
use crate::prefs::Prefs;
use crate::summarizer::Summarizer;
use crate::summarizers::{Choice, Probe};
use crate::theme::{self, color};
use crate::view_state::{TableRow, TreeRow, View, ViewState};

/// How often the shown session's database is polled; also one replay step while playing.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// How often the sessions list is reread.
const SESSIONS_POLL: Duration = Duration::from_secs(3);
/// Polls between advances of running durations while live: once a second.
const POLLS_PER_TICK: u64 = 4;
/// How long a new row stays highlighted.
const FRESH: Duration = Duration::from_secs(2);
/// How long "copied" shows on a copy button.
const COPIED_FOR: Duration = Duration::from_millis(1200);
/// Rows longer than this get a bold duration.
const LONG_MS: i64 = 10_000;
/// Height of every tree and table row.
const ROW_HEIGHT: f32 = 32.;
/// What "copied" shows on after an event is copied for an agent.
const FOR_AGENT: &str = "copy-for-agent";
/// What "copied" shows on after a row is copied from its menu.
const COPY_ROW: &str = "copy-row";

/// Key context of the viewer; single-key shortcuts only apply outside text inputs.
const CONTEXT: &str = "IchnosViewer";
const SHORTCUTS: &str = "IchnosViewer && !Input";

/// Where the viewer reads from and what it starts on.
#[derive(Debug)]
pub struct Options {
    /// The data directory; sessions are in its `sessions` folder.
    pub data_dir: PathBuf,
    /// The session to pin; `None` follows the newest.
    pub session: Option<SessionId>,
    /// The view to open on.
    pub view: View,
    /// The summarizer, unless summaries are off.
    pub summaries: Option<Summarizer>,
    /// The summarizer chosen at launch.
    pub summarizer: Choice,
    /// The summarizers listed in Settings.
    pub summarizers: Vec<Choice>,
    /// Checks which summarizers this Mac can run.
    pub probe: Probe,
    /// Search by meaning for the session switcher, unless it is off.
    pub search: Option<Search>,
    /// Requests from `ichnos open` and `ichnos stop`.
    pub listener: Option<Listener>,
}

/// How the graph sorts each card's tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolSort {
    Time,
    Count,
}

/// The viewer window's root view.
pub struct Viewer {
    focus: FocusHandle,
    data_dir: PathBuf,
    sessions_dir: PathBuf,
    summaries: Option<Summarizer>,
    summarizer: Choice,
    summarizers: Vec<Choice>,
    probe: Probe,
    search: Option<Search>,
    listener: Option<Listener>,
    catalog: Arc<Mutex<Catalog>>,
    sessions: Vec<SessionEntry>,
    session: Option<SessionId>,
    follow_newest: bool,
    live: Option<LiveSession>,
    state: ViewState,
    model: SessionModel,
    bounds: Option<Bounds>,
    rows: Vec<TreeRow>,
    table: Vec<TableRow>,
    /// The model's version: the live data's version it was built from.
    built_from: u64,
    filter_input: Entity<InputState>,
    tree_scroll: UniformListScrollHandle,
    table_scroll: UniformListScrollHandle,
    /// Entry key -> when it first appeared, for the fresh-row highlight; `None` for rows that
    /// were there on first paint.
    seen: HashMap<Key, Option<Instant>>,
    /// Polls since the viewer opened.
    polls: u64,
    /// The copy button that shows "copied", and since when.
    copied: Option<(SharedString, Instant)>,
    tool_sort: ToolSort,
    /// The pointer over the replay strip, as a fraction of its width, and the fraction a tick
    /// may be from it and still count as hovered.
    replay_hover: Option<(f32, f32)>,
    /// Where the replay strip was last painted.
    strip: std::rc::Rc<std::cell::Cell<Option<gpui_kit::Bounds<gpui_kit::Pixels>>>>,
    switcher: Option<Entity<Switcher>>,
    ask: Option<Entity<AskPanel>>,
    /// Whether the Ask panel focuses on the selected row.
    ask_focus: bool,
    /// Whether the keyboard shortcuts overlay is open.
    help: bool,
    /// The settings window, once opened.
    settings: Option<AnyWindowHandle>,
    subscriptions: Vec<Subscription>,
}

#[cfg(feature = "screenshots")]
impl Viewer {
    /// Put the viewer in a state worth a screenshot: `selected`, `turn`, `replay`, `switcher`, `ask` or
    /// `help`; anything else leaves it as opened.
    pub fn apply_shot(&mut self, shot: &str, window: &mut Window, cx: &mut Context<Self>) {
        match shot {
            "selected" => self.on_next_error(&NextError, window, cx),
            "turn" => {
                if let Some(&turn) = self.model.turns().last() {
                    let key = self.model.node(turn).key().clone();
                    self.select(key, false, cx);
                }
            }
            "replay" => {
                if let Some(b) = self.bounds {
                    self.seek(b.t0 + (b.t1 - b.t0) / 2, cx);
                }
            }
            "switcher" => self.toggle_switcher(window, cx),
            "ask" => self.toggle_ask(false, window, cx),
            "help" => self.toggle_help(cx),
            _ => {}
        }
    }
}

impl Viewer {
    /// The skill name as a compact metadata tag.
    pub(super) fn skill_tag(name: &str) -> AnyElement {
        div()
            .id("skill-tag")
            .flex_none()
            .max_w(px(140.))
            .px(px(5.))
            .py(px(1.))
            .line_height(px(14.))
            .border_1()
            .border_color(color(theme::PURPLE_LINE))
            .rounded(px(theme::R_TAG))
            .text_size(px(theme::FS_SMALL))
            .text_color(color(theme::PURPLE_TEXT))
            .truncate()
            .whitespace_nowrap()
            .bg(color(theme::PURPLE_BG))
            .child(name.to_owned())
            .test_support()
            .into_any_element()
    }

    /// A viewer on `options`, polling in the background.
    pub fn new(options: Options, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sessions_dir = options.data_dir.join(ichnos_core::SESSIONS_DIR);
        let filter_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter: is:error tool:Bash file:feed.rs agent:Explore")
        });
        let subscriptions = vec![
            cx.subscribe_in(&filter_input, window, Self::on_filter_input),
            cx.observe_window_appearance(window, |_, window, cx| {
                theme::apply_appearance(window.appearance(), cx);
                cx.notify();
            }),
        ];
        let mut viewer = Self {
            focus: cx.focus_handle(),
            catalog: Arc::new(Mutex::new(Catalog::new(sessions_dir.clone()))),
            sessions_dir,
            data_dir: options.data_dir,
            summaries: options.summaries,
            summarizer: options.summarizer,
            summarizers: options.summarizers,
            probe: options.probe,
            search: options.search,
            listener: options.listener,
            sessions: Vec::new(),
            follow_newest: options.session.is_none(),
            session: None,
            live: None,
            state: ViewState::new(options.view),
            model: SessionModel::default(),
            bounds: None,
            rows: Vec::new(),
            table: Vec::new(),
            built_from: 0,
            filter_input,
            tree_scroll: UniformListScrollHandle::new(),
            table_scroll: UniformListScrollHandle::new(),
            seen: HashMap::new(),
            polls: 0,
            copied: None,
            tool_sort: ToolSort::Time,
            replay_hover: None,
            strip: std::rc::Rc::default(),
            switcher: None,
            ask: None,
            ask_focus: false,
            help: false,
            settings: None,
            subscriptions,
        };
        if let Some(session) = options.session {
            viewer.show_session(session, cx);
        }
        viewer.focus.focus(window, cx);
        viewer.poll_sessions(cx);
        Self::poll_feed(window, cx);
        viewer
    }

    /// Show session `id` from now on, pinned unless `follow_newest` says otherwise.
    pub fn open_session(
        &mut self,
        id: Option<SessionId>,
        view: Option<View>,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = view {
            self.state.set_view(view);
        }
        if let Some(id) = id {
            self.follow_newest = false;
            self.show_session(id, cx);
        } else {
            self.follow_newest = true;
            if let Some(newest) = self.sessions.first().map(|s| s.id().clone()) {
                self.show_session(newest, cx);
            }
        }
        cx.notify();
    }

    fn show_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if self.session.as_ref() == Some(&id) {
            return;
        }
        let watch = self.summaries.as_ref().map(|host| host.watch(id.clone()));
        self.live = Some(LiveSession::open(&self.sessions_dir, id.clone(), watch));
        self.session = Some(id);
        self.state.reset_for_session();
        self.seen.clear();
        self.built_from = 0;
        self.refresh();
        cx.notify();
    }

    /// The shown session's data.
    fn data(&self) -> &SessionData {
        data_of(self.live.as_ref())
    }

    fn summaries(&self) -> &Summaries {
        self.data().summaries()
    }

    fn connection(&self) -> Option<Connection> {
        self.live.as_ref().map(LiveSession::connection)
    }

    /// Rebuild the model and rows from the data, the replay cursor and the filter.
    fn refresh(&mut self) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let moment = Moment {
            at: self.state.replay_at(),
            live: self.connection() == Some(Connection::Live),
            clock: now_ms,
        };
        let data = data_of(self.live.as_ref());
        self.bounds = session_bounds(data.events(), data.steps());
        self.model = build_session(data, moment);
        self.rows = self.state.tree_rows(&self.model, data.summaries());
        self.table = if self.state.view() == View::Table {
            self.state.table_rows(&self.model, data.summaries())
        } else {
            Vec::new()
        };
        let version = self.live.as_ref().map_or(0, LiveSession::version);
        let first_paint = self.seen.is_empty();
        for (_, node) in self.model.iter() {
            self.seen
                .entry(node.key().clone())
                .or_insert_with(|| (!first_paint).then(Instant::now));
        }
        if version != self.built_from {
            self.built_from = version;
            if self.state.tail() && self.state.replay_at().is_none() {
                self.tree_scroll.scroll_to_bottom();
                self.table_scroll.scroll_to_bottom();
            }
        }
    }

    /// Whether row `key` appeared within the last couple of seconds.
    fn is_fresh(&self, key: &Key) -> bool {
        self.seen
            .get(key)
            .is_some_and(|at| at.is_some_and(|at| at.elapsed() < FRESH))
    }

    /// The keys of the rows in the current view, in order.
    fn order(&self) -> Vec<Key> {
        match self.state.view() {
            View::Table => self
                .table
                .iter()
                .map(|row| row.call.key().clone())
                .collect(),
            _ => self
                .rows
                .iter()
                .map(|row| self.model.node(row.id).key().clone())
                .collect(),
        }
    }

    /// Scroll the row waiting to be revealed into view.
    fn scroll_to_pending(&mut self) {
        let Some(key) = self.state.take_pending_reveal() else {
            return;
        };
        let order = self.order();
        if let Some(index) = order.iter().position(|k| *k == key) {
            let handle = match self.state.view() {
                View::Table => &self.table_scroll,
                _ => &self.tree_scroll,
            };
            handle.scroll_to_item(index, ScrollStrategy::Center);
        }
    }

    fn select(&mut self, key: Key, scroll: bool, cx: &mut Context<Self>) {
        self.state.select(key, scroll);
        self.scroll_to_pending();
        cx.notify();
    }

    fn reveal(&mut self, key: &Key, cx: &mut Context<Self>) {
        self.state.reveal(&self.model, key);
        self.refresh();
        self.scroll_to_pending();
        cx.notify();
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.state.set_view(view);
        self.refresh();
        cx.notify();
    }

    fn set_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_input.update(cx, |input, cx| {
            input.set_value(query.to_owned(), window, cx);
        });
        self.state.set_query(query);
        self.refresh();
        cx.notify();
    }

    fn on_filter_input(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let query = input.read(cx).value().to_string();
                self.state.set_query(query);
                self.refresh();
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.focus.focus(window, cx),
            _ => {}
        }
    }

    /// Seek to the strip position under window x-coordinate `x`, clamped to the strip.
    fn scrub_to(&mut self, x: f32, cx: &mut Context<Self>) {
        let (Some(b), Some(rail)) = (self.bounds, self.strip.get()) else {
            return;
        };
        let width = f32::from(rail.size.width).max(1.);
        let at = ((x - f32::from(rail.origin.x)) / width).clamp(0., 1.);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            reason = "a pixel fraction of a millisecond span"
        )]
        let ms = b.t0 + (at * (b.t1 - b.t0) as f32).round() as i64;
        self.seek(ms, cx);
    }

    fn set_all_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.state.set_all_open(&self.model, open);
        self.refresh();
        cx.notify();
    }

    fn seek(&mut self, ms: i64, cx: &mut Context<Self>) {
        self.state.pause();
        self.state.seek(self.bounds, ms);
        self.refresh();
        cx.notify();
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.state.pause();
        let data = self.data();
        let all = moments(data.events(), data.steps());
        self.state.step(&all, forward);
        self.refresh();
        cx.notify();
    }

    fn toggle_panel(&mut self, cx: &mut Context<Self>) {
        self.state.panel_mut().toggle();
        cx.notify();
    }

    fn toggle_help(&mut self, cx: &mut Context<Self>) {
        self.help = !self.help;
        cx.notify();
    }

    /// Bring the settings window forward, opening it the first time. Deferred until this update
    /// ends: the new window draws at once, and its first draw reads the viewer.
    fn open_settings(cx: &mut Context<Self>) {
        let viewer = cx.entity();
        cx.defer(move |cx| {
            let open = viewer.read(cx).settings;
            if let Some(window) = open
                && window
                    .update(cx, |_, window, _| window.activate_window())
                    .is_ok()
            {
                return;
            }
            let window = crate::settings::open(&viewer, cx);
            viewer.update(cx, |viewer, _| viewer.settings = window);
        });
    }

    /// Whether the viewer switches to the newest session as one starts.
    pub(crate) fn follows_newest(&self) -> bool {
        self.follow_newest
    }

    /// Follow the newest session, or stay on the one shown.
    pub(crate) fn set_follow_newest(&mut self, follow: bool, cx: &mut Context<Self>) {
        if follow {
            self.open_session(None, None, cx);
        } else {
            self.follow_newest = false;
            cx.notify();
        }
    }

    /// The summarizer in use, or `None` when summaries were turned off at launch.
    pub(crate) fn summarizer(&self) -> Option<&Choice> {
        self.summaries.as_ref().map(|_| &self.summarizer)
    }

    /// The summarizers to offer, and how to check whether each can run.
    pub(crate) fn summarizers(&self) -> (&[Choice], Probe) {
        (&self.summarizers, self.probe)
    }

    /// Summarize with `choice` from the next batch on, and remember it for the next launch
    /// (unless it came from `--summarizer`).
    pub(crate) fn set_summarizer(&mut self, choice: Choice, cx: &mut Context<Self>) {
        if let Some(summaries) = &self.summaries {
            summaries.set_model(choice.model());
        }
        if let Some(key) = choice.key() {
            let mut prefs = Prefs::load(&self.data_dir);
            prefs.summarizer = Some(key.to_owned());
            if let Err(err) = prefs.save(&self.data_dir) {
                ichnos_core::error_log::append(&self.data_dir, &format!("settings: {err:?}"));
            }
        }
        self.summarizer = choice;
        cx.notify();
    }

    fn toggle_play(&mut self, cx: &mut Context<Self>) {
        self.state.toggle_play(self.bounds);
        self.refresh();
        cx.notify();
    }

    fn go_live(&mut self, cx: &mut Context<Self>) {
        self.state.go_live();
        self.refresh();
        cx.notify();
    }

    fn toggle_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }
        let sessions = self.sessions.clone();
        let current = self.session.clone();
        let search = self.search.clone();
        let switcher = cx.new(|cx| Switcher::new(sessions, current, search, window, cx));
        self.subscriptions.push(cx.subscribe_in(
            &switcher,
            window,
            |this, _, event, window, cx| {
                match event {
                    SwitcherEvent::Pick(id) => this.open_session(Some(id.clone()), None, cx),
                    SwitcherEvent::Close => {}
                }
                this.switcher = None;
                this.focus.focus(window, cx);
                cx.notify();
            },
        ));
        self.switcher = Some(switcher);
        cx.notify();
    }

    fn toggle_ask(&mut self, focus_selected: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.ask.take().is_some() && !focus_selected {
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.ask_focus = focus_selected;
        let ask = cx.new(|cx| AskPanel::new(window, cx));
        self.subscriptions.push(cx.subscribe_in(
            &ask,
            window,
            |this, _, _: &ask_panel::Close, window, cx| {
                this.ask = None;
                this.focus.focus(window, cx);
                cx.notify();
            },
        ));
        let question = ask.read(cx).question().clone();
        self.subscriptions
            .push(cx.subscribe(&question, |_, _, _: &InputEvent, cx| cx.notify()));
        self.ask = Some(ask);
        cx.notify();
    }

    /// Apply a zoom change when the graph is shown.
    fn zoom_graph(
        &mut self,
        change: impl FnOnce(&mut crate::view_state::Pan),
        cx: &mut Context<Self>,
    ) {
        if self.state.view() == View::Graph {
            change(self.state.graph_pan_mut());
            cx.notify();
        }
    }
}

impl Viewer {
    /// The draggable divider between the main pane and the right-hand panel; double-click
    /// resets the panel's width.
    fn render_panel_handle(cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("panel-handle")
            .role(accesskit::Role::Splitter)
            .aria_label("Resize inspector")
            .flex_none()
            .w(px(crate::view_state::HANDLE_WIDTH))
            .border_l_1()
            .border_color(color(theme::LINE))
            .cursor_col_resize()
            .hover(|style| style.bg(color(theme::LINE)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        this.state.panel_mut().reset();
                    } else {
                        this.state.panel_mut().start(f32::from(event.position.x));
                    }
                    cx.notify();
                }),
            )
            .test_support()
    }

    fn end_panel_drag(&mut self, cx: &mut Context<Self>) {
        self.state.set_scrubbing(false);
        if self.state.panel().is_dragging() {
            self.state.panel_mut().end();
            cx.notify();
        }
    }
}

impl Focusable for Viewer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match &self.session {
            Some(id) => format!("Ichnos · {}", short_id(id.as_str())),
            None => "Ichnos".to_owned(),
        };
        window.set_window_title(&title);

        let view = self.state.view();
        let content = match view {
            View::Graph => self.render_graph(cx),
            View::Table => self.render_table(window, cx),
            View::Tree => self.render_tree(cx),
        };
        let panel = self.state.panel().is_open().then(|| match view {
            View::Graph => self.render_agent_inspector(cx),
            View::Tree | View::Table => self.render_inspector(window, cx).into_any_element(),
        });
        let main =
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(self.render_stats(
                            f32::from(window.viewport_size().width) < toolbar::NARROW,
                        ))
                        .child(content),
                )
                .when_some(panel, |d, panel| {
                    d.child(Self::render_panel_handle(cx)).child(panel)
                });

        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_move_down))
            .on_action(cx.listener(Self::on_move_up))
            .on_action(cx.listener(Self::on_toggle_row))
            .on_action(cx.listener(Self::on_next_error))
            .on_action(cx.listener(|this, _: &TogglePlay, _, cx| this.toggle_play(cx)))
            .on_action(cx.listener(|this, _: &StepBack, _, cx| this.step(false, cx)))
            .on_action(cx.listener(|this, _: &StepForward, _, cx| this.step(true, cx)))
            .on_action(cx.listener(|this, _: &TogglePanel, _, cx| this.toggle_panel(cx)))
            .on_action(cx.listener(|this, _: &ToggleHelp, _, cx| this.toggle_help(cx)))
            .on_action(cx.listener(|_, _: &OpenSettings, _, cx| Self::open_settings(cx)))
            .on_action(cx.listener(|this, _: &ShowTree, _, cx| this.set_view(View::Tree, cx)))
            .on_action(cx.listener(|this, _: &ShowGraph, _, cx| this.set_view(View::Graph, cx)))
            .on_action(cx.listener(|this, _: &ShowTable, _, cx| this.set_view(View::Table, cx)))
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter_input
                    .update(cx, |input, cx| input.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &ToggleSwitcher, window, cx| {
                this.toggle_switcher(window, cx);
            }))
            .on_action(cx.listener(Self::on_dismiss))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.state.scrubbing() && event.pressed_button == Some(MouseButton::Left) {
                    this.scrub_to(f32::from(event.position.x), cx);
                }
                let window = f32::from(window.viewport_size().width);
                if this
                    .state
                    .panel_mut()
                    .drag_to(f32::from(event.position.x), window)
                {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_panel_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_panel_drag(cx)),
            )
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.zoom_graph(|pan| pan.zoom_by(graph::ZOOM_STEP), cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.zoom_graph(|pan| pan.zoom_by(1. / graph::ZOOM_STEP), cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| {
                this.zoom_graph(crate::view_state::Pan::reset_zoom, cx);
            }))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(color(theme::BG))
            .text_color(color(theme::FG))
            .text_size(px(theme::FS_BASE))
            // Lets the inspector's text blocks be selected with the mouse and copied.
            .child(gpui_kit::base::TextSelectionLayer)
            .child(self.render_toolbar(window, cx))
            .child(self.render_replay(window, cx))
            .child(main)
            .children(self.ask.clone().map(|ask| self.render_ask(&ask, cx)))
            .when(self.help, |d| d.child(Self::render_help(cx)))
            .children(self.switcher.clone())
    }
}

/// `live`'s data, or no data.
fn data_of(live: Option<&LiveSession>) -> &SessionData {
    static EMPTY: std::sync::LazyLock<SessionData> = std::sync::LazyLock::new(SessionData::default);
    live.map_or(&EMPTY, LiveSession::data)
}

/// The first eight characters of a session id.
fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
