//! The `⌘K` session switcher: every session with its first prompt; type to filter, arrows and
//! Enter (or a click) to open one.

use accesskit::Role;
use gpui_kit::component::input::{self, Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, ScrollStrategy,
    StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle, Window,
    div, px, relative, uniform_list,
};
use ichnos_core::catalog::{SessionEntry, filter_sessions};
use ichnos_core::store::SessionId;

use super::short_id;
use crate::theme::{self, MONO, color, color_alpha};

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
    _subscription: Subscription,
}

impl EventEmitter<SwitcherEvent> for Switcher {}

impl Switcher {
    /// A switcher over `sessions`, marking `current`, with the search box focused.
    pub fn new(
        sessions: Vec<SessionEntry>,
        current: Option<SessionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search sessions by prompt, folder or id")
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription =
            cx.subscribe(
                &input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => {
                        this.cursor = 0;
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
            _subscription: subscription,
        }
    }

    /// Replace the listed sessions, e.g. after the list was reread.
    pub fn set_sessions(&mut self, sessions: Vec<SessionEntry>, cx: &mut Context<Self>) {
        self.sessions = sessions;
        cx.notify();
    }

    fn found(&self, cx: &Context<Self>) -> Vec<SessionEntry> {
        let query = self.input.read(cx).value().to_string();
        filter_sessions(&self.sessions, &query).cloned().collect()
    }

    fn pick_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self.found(cx).get(index) {
            cx.emit(SwitcherEvent::Pick(entry.id().clone()));
        }
    }

    fn move_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.found(cx).len();
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
        let found = self.found(cx);
        let total = self.sessions.len();
        let shown = found.len();
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
                                let found = this.found(cx);
                                range
                                    .filter_map(|ix| {
                                        let entry = found.get(ix)?.clone();
                                        Some(session_row(
                                            &entry,
                                            ix,
                                            ix == cursor,
                                            current.as_ref() == Some(entry.id()),
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
                            .child(div().ml_auto().child(format!("{shown} of {total}"))),
                    ),
            )
    }
}

fn session_row(
    entry: &SessionEntry,
    ix: usize,
    on: bool,
    current: bool,
    now_ms: u128,
    cx: &mut Context<Switcher>,
) -> gpui_kit::AnyElement {
    let description = entry.overview().description.clone();
    let errors = entry.overview().errors;
    div()
        .id(("session", ix))
        .role(Role::Button)
        .aria_label(description.as_deref().map_or_else(
            || "Session".to_owned(),
            |text| format!("Session: {}", super::one_line(text)),
        ))
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
        .child(
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
        .into_any_element()
}
