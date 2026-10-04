//! The `⌘K` session switcher: every session with its first prompt; type to filter, arrows and
//! Enter (or a click) to open one. Below the sessions whose words match, it lists sessions whose
//! prompts, errors or summaries are close in meaning, found by `ichnos-search`.

use std::time::Duration;

use accesskit::Role;
use gpui_kit::component::input::{self, Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, ScrollStrategy,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, UniformListScrollHandle,
    Window, div, px, relative, uniform_list,
};
use ichnos_core::catalog::{SessionEntry, filter_sessions};
use ichnos_core::store::SessionId;
use ichnos_search::{Search, SearchState, SessionMatch};

use super::short_id;
use crate::theme::{self, MONO, color, color_alpha};
use crate::view_state::related_matches;

/// How long typing must pause before a search by meaning runs.
const SEARCH_DELAY: Duration = Duration::from_millis(150);

/// Shortest query searched by meaning, in characters; shorter ones match too much.
const MIN_SEARCH_CHARS: usize = 3;

/// One row of the list.
#[derive(Debug, Clone)]
enum Row {
    /// A session whose words match the query.
    Session(SessionEntry),
    /// A session found by meaning, with the text that matched.
    Related(SessionEntry, SessionMatch),
}

impl Row {
    fn entry(&self) -> &SessionEntry {
        match self {
            Self::Session(entry) | Self::Related(entry, _) => entry,
        }
    }
}

/// What the switcher asks of the viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitcherEvent {
    /// Show this session.
    Pick(SessionId),
    /// Close without picking.
    Close,
}

/// The session switcher overlay.
pub struct Switcher {
    input: Entity<InputState>,
    sessions: Vec<SessionEntry>,
    current: Option<SessionId>,
    cursor: usize,
    scroll: UniformListScrollHandle,
    search: Option<Search>,
    /// Sessions found by meaning for `related_query`.
    related: Vec<SessionMatch>,
    related_query: String,
    /// The pending search; replacing it cancels the previous one.
    search_task: Option<Task<()>>,
    _subscription: Subscription,
}

impl EventEmitter<SwitcherEvent> for Switcher {}

impl Switcher {
    /// A switcher over `sessions`, marking `current`, with the search box focused. With `search`
    /// it also lists sessions close in meaning to the query.
    pub fn new(
        sessions: Vec<SessionEntry>,
        current: Option<SessionId>,
        search: Option<Search>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = if search.is_some() {
            "Search sessions by prompt, folder, id or meaning"
        } else {
            "Search sessions by prompt, folder or id"
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription =
            cx.subscribe(
                &input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => {
                        this.cursor = 0;
                        this.search_by_meaning(cx);
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.pick_at(this.cursor, cx),
                    _ => {}
                },
            );
        Self {
            input,
            sessions,
            current,
            cursor: 0,
            scroll: UniformListScrollHandle::new(),
            search,
            related: Vec::new(),
            related_query: String::new(),
            search_task: None,
            _subscription: subscription,
        }
    }

    /// Replace the listed sessions, e.g. after the list was reread.
    pub fn set_sessions(&mut self, sessions: Vec<SessionEntry>, cx: &mut Context<Self>) {
        self.sessions = sessions;
        cx.notify();
    }

    fn query(&self, cx: &Context<Self>) -> String {
        self.input.read(cx).value().trim().to_owned()
    }

    /// Sessions whose words match, then the ones found by meaning for the current query.
    fn rows(&self, cx: &Context<Self>) -> Vec<Row> {
        let query = self.query(cx);
        let found: Vec<&SessionEntry> = filter_sessions(&self.sessions, &query).collect();
        let mut rows: Vec<Row> = found
            .iter()
            .map(|&entry| Row::Session(entry.clone()))
            .collect();
        if self.related_query == query {
            let listed: Vec<&SessionId> = found.iter().map(|entry| entry.id()).collect();
            let known: Vec<&SessionId> = self.sessions.iter().map(SessionEntry::id).collect();
            rows.extend(
                related_matches(&self.related, &listed, &known)
                    .into_iter()
                    .filter_map(|found| {
                        let entry = self.sessions.iter().find(|e| *e.id() == found.session)?;
                        Some(Row::Related(entry.clone(), found))
                    }),
            );
        }
        rows
    }

    /// Search by meaning once typing pauses, replacing any search still pending.
    fn search_by_meaning(&mut self, cx: &mut Context<Self>) {
        let Some(search) = self.search.clone() else {
            return;
        };
        let query = self.query(cx);
        if query.chars().count() < MIN_SEARCH_CHARS {
            self.search_task = None;
            self.related.clear();
            self.related_query = query;
            return;
        }
        self.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let text = query.clone();
            let found = cx
                .background_executor()
                .spawn(async move { search.query(&text) })
                .await;
            // Until the model is ready, or if the search fails, only word matches are listed.
            let _ = this.update(cx, |this, cx| {
                this.related = found.unwrap_or_default();
                this.related_query = query;
                cx.notify();
            });
        }));
    }

    /// What the footer says about search by meaning, when it isn't simply working.
    fn search_status(&self) -> Option<&'static str> {
        match self.search.as_ref()?.state() {
            SearchState::Ready => None,
            SearchState::Loading => Some("loading meaning search…"),
            SearchState::Failed(_) => Some("meaning search off"),
        }
    }

    fn pick_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows(cx).get(index) {
            cx.emit(SwitcherEvent::Pick(row.entry().id().clone()));
        }
    }

    fn move_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.rows(cx).len();
        if count == 0 {
            return;
        }
        self.cursor = self.cursor.saturating_add_signed(delta).min(count - 1);
        self.scroll
            .scroll_to_item(self.cursor, ScrollStrategy::Center);
        cx.notify();
    }
}

impl Render for Switcher {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let found = self.rows(cx);
        let total = self.sessions.len();
        let shown = found.len();
        let related = found
            .iter()
            .filter(|row| matches!(row, Row::Related(..)))
            .count();
        let count = if related > 0 {
            format!("{} of {total} + {related} related", shown - related)
        } else {
            format!("{shown} of {total}")
        };
        let status = self.search_status();
        self.cursor = self.cursor.min(shown.saturating_sub(1));
        let now_ms = u128::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or_default();

        div()
            .id("switcher-overlay")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .items_start()
            .pt(relative(0.12))
            .bg(color_alpha(0x080a_0d99))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(SwitcherEvent::Close)),
            )
            .on_action(cx.listener(|_, _: &input::Escape, _, cx| cx.emit(SwitcherEvent::Close)))
            .on_action(cx.listener(|this, _: &input::MoveDown, _, cx| this.move_cursor(1, cx)))
            .on_action(cx.listener(|this, _: &input::MoveUp, _, cx| this.move_cursor(-1, cx)))
            .child(
                div()
                    .id("switcher")
                    .w(px(640.))
                    .max_h(px(560.))
                    .flex()
                    .flex_col()
                    .bg(color(theme::CARD))
                    .border_1()
                    .border_color(color(theme::LINE_2))
                    .rounded(px(theme::R_POPUP))
                    .shadow_lg()
                    .overflow_hidden()
                    // Clicks inside don't close the overlay.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(color(theme::LINE))
                            .text_size(px(theme::FS_TITLE))
                            .child(Input::new(&self.input).appearance(false).cleanable(false)),
                    )
                    .child(if found.is_empty() {
                        div()
                            .p_6()
                            .flex()
                            .justify_center()
                            .text_color(color(theme::DIM))
                            .child(if total > 0 {
                                "No sessions match"
                            } else {
                                "No sessions yet"
                            })
                            .into_any_element()
                    } else {
                        let current = self.current.clone();
                        let cursor = self.cursor;
                        uniform_list(
                            "sessions",
                            shown,
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                let found = this.rows(cx);
                                range
                                    .filter_map(|ix| {
                                        let row = found.get(ix)?;
                                        Some(session_row(
                                            row,
                                            ix,
                                            ix == cursor,
                                            current.as_ref() == Some(row.entry().id()),
                                            now_ms,
                                            cx,
                                        ))
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(&self.scroll)
                        .h(px(420.))
                        .p_1()
                        .into_any_element()
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(14.))
                            .px(px(14.))
                            .py_2()
                            .border_t_1()
                            .border_color(color(theme::LINE))
                            .font_family(MONO)
                            .text_size(px(theme::FS_TINY))
                            .text_color(color(theme::DIM))
                            .child("↑↓ choose")
                            .child("↵ open")
                            .child("esc close")
                            .child(div().ml_auto().child(count))
                            .children(status),
                    ),
            )
    }
}

fn session_row(
    row: &Row,
    ix: usize,
    on: bool,
    current: bool,
    now_ms: u128,
    cx: &mut Context<Switcher>,
) -> gpui_kit::AnyElement {
    let entry = row.entry();
    let matched = match row {
        Row::Session(_) => None,
        Row::Related(_, found) => Some(found.clone()),
    };
    let description = entry.overview().description.clone();
    let errors = entry.overview().errors;
    let session = description.as_deref().map_or_else(
        || "Session".to_owned(),
        |text| format!("Session: {}", super::one_line(text)),
    );
    let label = match &matched {
        Some(found) => format!("{session}. Related {}: {}", found.kind, found.text),
        None => session,
    };
    div()
        .id(("session", ix))
        .role(Role::Button)
        .aria_label(label)
        // Rows are laid out one by one; without a width each fits its own text
        // and the columns stop lining up.
        .w_full()
        .h(px(56.))
        .px_3()
        .py_2()
        .rounded(px(theme::R_CARD))
        .border_l_2()
        .border_color(color(if on { theme::BLUE } else { theme::CARD }))
        .cursor_pointer()
        .when(on, |d| d.bg(color(theme::SEL)))
        .on_mouse_move(cx.listener(move |this, _, _, cx| {
            if this.cursor != ix {
                this.cursor = ix;
                cx.notify();
            }
        }))
        .on_click(cx.listener(move |this, _, _, cx| this.pick_at(ix, cx)))
        .child(
            div()
                .truncate()
                .text_size(px(theme::FS_BASE))
                .when_some(description.clone(), |d, text| {
                    d.text_color(color(theme::BRIGHT))
                        .child(super::one_line(&text))
                })
                .when(description.is_none(), |d| {
                    d.italic()
                        .text_color(color(theme::DIM))
                        .child("No prompt yet")
                }),
        )
        .when_some(matched.clone(), |d, found| {
            // Same height as a session row: the matched text takes the facts' line.
            d.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(10.))
                    .mt(px(3.))
                    .text_size(px(theme::FS_TINY))
                    .child(
                        div()
                            .flex_none()
                            .font_family(MONO)
                            .text_color(color(theme::BLUE_TEXT))
                            .child(format!("related {}", found.kind)),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_color(color(theme::MUTED))
                            .child(super::one_line(&found.text)),
                    ),
            )
        })
        .when(matched.is_none(), |d| {
            d.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(10.))
                    .mt(px(3.))
                    .font_family(MONO)
                    .text_size(px(theme::FS_TINY))
                    .text_color(color(theme::MUTED))
                    .when(entry.is_recent(now_ms), |d| {
                        d.child(div().text_color(color(theme::LIVE_TEXT)).child("live"))
                    })
                    .when(current, |d| {
                        d.child(div().text_color(color(theme::BLUE_TEXT)).child("viewing"))
                    })
                    .children(entry.facts())
                    .when(errors > 0, |d| {
                        d.child(
                            div()
                                .text_color(color(theme::RED_TEXT))
                                .child(format!("{errors} failed")),
                        )
                    })
                    .when_some(entry.error().map(str::to_owned), |d, error| {
                        d.child(
                            div()
                                .text_color(color(theme::RED_TEXT))
                                .truncate()
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .ml_auto()
                            .text_color(color(theme::DIM))
                            .font_weight(FontWeight::NORMAL)
                            .child(short_id(entry.id().as_str()).to_owned()),
                    ),
            )
        })
        .into_any_element()
}
