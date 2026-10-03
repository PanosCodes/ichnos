//! The toolbar: session picker, view switch, filter, error jump, expand/collapse, Ask and the
//! connection pill; plus the stats line and the key bar around the rows.

use gpui_kit::component::input::{self, Input};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use ichnos_core::catalog::project_of;
use ichnos_model::labels::{duration, tokens};

use super::{Viewer, short_id};
use crate::live::Connection;
use crate::present::plural;
use crate::theme::{self, MONO, color};
use crate::view_state::{View, failed_calls};

impl Viewer {
    pub(super) fn render_toolbar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = self
            .session
            .as_ref()
            .and_then(|id| self.sessions.iter().find(|s| s.id() == id));
        let cwd = cwd_of(self);
        let project = project_of(&cwd);
        let when = current
            .and_then(|s| i64::try_from(s.mtime_ms()).ok())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string())
            .unwrap_or_default();
        let picker_label = self.session.as_ref().map_or_else(
            || "no session".to_owned(),
            |id| short_id(id.as_str()).to_owned(),
        );
        let picker_extra = [project.as_str(), when.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .fold(String::new(), |extra, part| extra + " · " + part);

        let errors = failed_calls(&self.model).len();
        let (pill_background, pill_color, pill_dot, pill_text) = if self.state.replay_at().is_some()
        {
            (theme::SEL, theme::BLUE_TEXT, theme::BLUE, "Replay")
        } else {
            match self.connection() {
                Some(Connection::Live) => (theme::LIVE_BG, theme::LIVE_TEXT, theme::GREEN, "Live"),
                Some(Connection::Retry) => (theme::SEG, theme::AMBER, theme::AMBER, "Reconnecting"),
                Some(Connection::Connecting) => {
                    (theme::SEG, theme::MUTED, theme::DIM, "Connecting")
                }
                None => (theme::SEG, theme::MUTED, theme::DIM, "No sessions"),
            }
        };
        let show_model = self.state.show_model();
        let has_query = !self.state.query().is_empty();

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .h(px(42.))
            .flex_none()
            .px_3()
            .border_b_1()
            .border_color(color(theme::LINE))
            .bg(color(theme::BAR_BG))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(theme::FS_BASE))
                    .text_color(color(theme::BRIGHT))
                    .mr_1()
                    .child("Ichnos"),
            )
            .child(
                div()
                    .id("picker")
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .h(px(theme::CONTROL_HEIGHT))
                    .px_2()
                    .border_1()
                    .border_color(color(theme::LINE_2))
                    .rounded(px(theme::R_CONTROL))
                    .font_family(MONO)
                    .text_size(px(theme::FS_SMALL))
                    .text_color(color(theme::TEXT_2))
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .hover(|s| {
                        s.text_color(color(theme::BRIGHT))
                            .border_color(color(theme::HOVER_LINE))
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_switcher(window, cx)))
                    .child(picker_label)
                    .child(div().text_color(color(theme::DIM)).child(picker_extra))
                    .child(kbd("⌘K")),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .bg(color(theme::SEG))
                    .border_1()
                    .border_color(color(theme::LINE_2))
                    .rounded(px(theme::R_CONTROL))
                    .p(px(2.))
                    .children(View::ALL.map(|view| {
                        let on = self.state.view() == view;
                        div()
                            .id(view.label())
                            .px(px(10.))
                            .py(px(3.))
                            .rounded(px(theme::R_TAG))
                            .cursor_pointer()
                            .when_else(
                                on,
                                |d| d.bg(color(theme::SEG_ON)).text_color(color(theme::WHITE)),
                                |d| {
                                    d.text_color(color(theme::MUTED))
                                        .hover(|s| s.text_color(color(theme::FG)))
                                },
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.set_view(view, cx)))
                            .child(view.label())
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w(px(120.))
                    .items_center()
                    .gap_1p5()
                    .h(px(28.))
                    .px_1p5()
                    .border_1()
                    .border_color(color(theme::LINE_2))
                    .rounded(px(theme::R_CONTROL))
                    .bg(color(theme::BG))
                    .on_action(cx.listener(|this, _: &input::Escape, window, cx| {
                        this.focus.focus(window, cx);
                    }))
                    .child(
                        div()
                            .id("model-chip")
                            .flex_none()
                            .px_1p5()
                            .rounded(px(theme::R_TAG))
                            .font_family(MONO)
                            .text_size(px(theme::FS_TINY))
                            .whitespace_nowrap()
                            .cursor_pointer()
                            .when_else(
                                show_model,
                                |d| {
                                    d.bg(color(theme::SEL))
                                        .text_color(color(0x00c3_d4f0))
                                        .child("+model steps ×")
                                },
                                |d| {
                                    d.border_1()
                                        .border_dashed()
                                        .border_color(color(theme::LINE_2))
                                        .text_color(color(theme::DIM))
                                        .child("+ model steps")
                                },
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.state.set_show_model(!show_model);
                                this.refresh();
                                cx.notify();
                            })),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.filter_input)
                                .appearance(false)
                                .cleanable(false),
                        ),
                    )
                    .child(if has_query {
                        div()
                            .id("clear-filter")
                            .cursor_pointer()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.set_query("", window, cx)),
                            )
                            .child(kbd("×"))
                            .into_any_element()
                    } else {
                        kbd("/").into_any_element()
                    }),
            )
            .child(
                div()
                    .id("errors")
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(theme::CONTROL_HEIGHT))
                    .px(px(10.))
                    .border_1()
                    .rounded(px(theme::R_CONTROL))
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .when_else(
                        errors > 0,
                        |d| {
                            d.border_color(color(theme::RED_LINE))
                                .text_color(color(theme::RED_TEXT))
                        },
                        |d| {
                            d.border_color(color(theme::LINE_2))
                                .text_color(color(theme::DIM))
                        },
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_next_error(&super::NextError, window, cx);
                    }))
                    .child(if errors > 0 {
                        format!("{} · next ↓", plural(errors, "error"))
                    } else {
                        "no errors".to_owned()
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .border_1()
                    .border_color(color(theme::LINE_2))
                    .rounded(px(theme::R_CONTROL))
                    .text_color(color(theme::BTN))
                    .text_size(px(theme::FS_SMALL))
                    .child(
                        div()
                            .id("expand")
                            .px_2()
                            .py_1()
                            .cursor_pointer()
                            .hover(|s| s.text_color(color(theme::BRIGHT)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.set_all_open(&this.model, true);
                                this.refresh();
                                cx.notify();
                            }))
                            .child("Expand"),
                    )
                    .child(
                        div()
                            .id("collapse")
                            .px_2()
                            .py_1()
                            .border_l_1()
                            .border_color(color(theme::LINE_2))
                            .cursor_pointer()
                            .hover(|s| s.text_color(color(theme::BRIGHT)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.set_all_open(&this.model, false);
                                this.refresh();
                                cx.notify();
                            }))
                            .child("Collapse"),
                    ),
            )
            .child(
                div()
                    .id("ask")
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(theme::CONTROL_HEIGHT))
                    .px(px(10.))
                    .border_1()
                    .border_color(color(theme::BLUE_LINE))
                    .rounded(px(theme::R_CONTROL))
                    .text_color(color(theme::BLUE_TEXT))
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .when(self.ask.is_some(), |d| d.bg(color(theme::SEL)))
                    .hover(|s| s.bg(color(theme::SEL)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let has_selection = this.state.selected().is_some();
                        if this.ask.is_some() {
                            this.toggle_ask(false, window, cx);
                        } else {
                            this.toggle_ask(has_selection, window, cx);
                        }
                    }))
                    .child("Ask agent"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .h(px(theme::CONTROL_HEIGHT))
                    .px(px(10.))
                    .rounded(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_nowrap()
                    .bg(color(pill_background))
                    .text_color(color(pill_color))
                    .child(div().size(px(6.)).rounded(px(3.)).bg(color(pill_dot)))
                    .child(pill_text),
            )
    }

    pub(super) fn render_stats(&self) -> impl IntoElement {
        let bar = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(14.))
            .h(px(28.))
            .px(px(14.))
            .border_b_1()
            .border_color(color(theme::SOFT_LINE))
            .text_color(color(theme::MUTED))
            .font_family(MONO)
            .text_size(px(theme::FS_SMALL))
            .whitespace_nowrap()
            .overflow_hidden();
        if self.data().events().is_empty() {
            return bar.child("Waiting for events…");
        }
        let stats = self.model.stats();
        let turns: Vec<_> = self
            .model
            .turns()
            .iter()
            .map(|&t| self.model.node(t).totals())
            .collect();
        let input: i64 = turns.iter().map(|t| t.input_tokens).sum();
        let tool_ms: i64 = turns.iter().map(|t| t.tool_ms).sum();
        bar.child(format!(
            "{} turns · {} calls · {}",
            stats.turns,
            stats.calls,
            plural(stats.subagents, "subagent")
        ))
        .when(stats.running > 0, |d| {
            d.child(
                div()
                    .text_color(color(theme::BLUE_TEXT))
                    .child(format!("{} running", stats.running)),
            )
        })
        .when(stats.errors > 0, |d| {
            d.child(
                div()
                    .text_color(color(theme::RED_TEXT))
                    .child(plural(stats.errors, "error")),
            )
        })
        .child(format!(
            "{} out / {} in",
            tokens(stats.output_tokens),
            tokens(input)
        ))
        .child(div().ml_auto().text_color(color(theme::FG)).child(format!(
            "model {} · tools {}",
            duration(stats.model_ms),
            duration(tool_ms)
        )))
    }

    pub(super) fn render_keybar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hint = |key: &'static str, what: &'static str| {
            div()
                .flex()
                .flex_row()
                .gap_1()
                .child(
                    div()
                        .font_family(MONO)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color(theme::BTN))
                        .child(key),
                )
                .child(what)
        };
        let follow = self.follow_newest;
        let tail = self.state.tail();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_4()
            .h(px(28.))
            .px(px(14.))
            .border_t_1()
            .border_color(color(theme::LINE))
            .text_color(color(theme::DIM))
            .text_size(px(theme::FS_SMALL))
            .whitespace_nowrap()
            .overflow_hidden()
            .child(hint("j k", "move"))
            .child(hint("↵", "expand"))
            .child(hint("e", "next error"))
            .child(hint("space", "play"))
            .child(hint("g", "graph"))
            .child(hint("/", "filter"))
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .child(
                        div()
                            .id("follow")
                            .cursor_pointer()
                            .when_else(
                                follow,
                                |d| d.text_color(color(theme::BLUE_TEXT)),
                                |d| d.hover(|s| s.text_color(color(theme::FG))),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if follow {
                                    this.follow_newest = false;
                                    cx.notify();
                                } else {
                                    this.open_session(None, None, cx);
                                }
                            }))
                            .child(if follow {
                                "following newest"
                            } else {
                                "pinned to this session"
                            }),
                    )
                    .child("·")
                    .child(
                        div()
                            .id("tail")
                            .cursor_pointer()
                            .when_else(
                                tail,
                                |d| d.text_color(color(theme::BLUE_TEXT)),
                                |d| d.hover(|s| s.text_color(color(theme::FG))),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.state.set_tail(!tail);
                                if !tail {
                                    this.tree_scroll.scroll_to_bottom();
                                    this.table_scroll.scroll_to_bottom();
                                }
                                cx.notify();
                            }))
                            .child(if tail {
                                "auto-scroll on"
                            } else {
                                "auto-scroll off"
                            }),
                    ),
            )
    }
}

/// A small key hint box.
pub(super) fn kbd(text: &'static str) -> gpui_kit::Div {
    div()
        .flex_none()
        .font_family(MONO)
        .text_size(px(theme::FS_TINY))
        .text_color(color(theme::DIM))
        .border_1()
        .border_color(color(theme::LINE_2))
        .px_1()
        .rounded(px(theme::R_TAG))
        .child(text)
}

/// The working directory the session recorded, if any event carries it.
pub(super) fn cwd_of(viewer: &Viewer) -> String {
    viewer
        .data()
        .events()
        .iter()
        .find_map(|event| event.extra("cwd").and_then(serde_json::Value::as_str))
        .unwrap_or_default()
        .to_owned()
}
