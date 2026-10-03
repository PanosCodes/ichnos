//! The tree: one row per prompt, expandable into calls, model steps and subagents.

use std::ops::Range;

use gpui_kit::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    ScrollWheelEvent, StatefulInteractiveElement as _, Styled as _, div, px, relative,
    uniform_list,
};
use ichnos_model::labels::{ChipClass, duration, label, tokens};
use ichnos_model::model::{Node, NodeKind};

use super::{LONG_MS, ROW_HEIGHT, Viewer, one_line};
use crate::present::{Tone, look, row_tokens};
use crate::theme::{self, MONO, color, tone_bar, tone_icon, tone_text};
use crate::view_state::TreeRow;

impl Viewer {
    pub(super) fn render_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.rows.is_empty() {
            let text = if self.state.filter().is_active() {
                "Nothing matches the filter."
            } else {
                "No entries yet."
            };
            return empty(text).into_any_element();
        }
        uniform_list(
            "tree",
            self.rows.len(),
            cx.processor(|this, range: Range<usize>, _window, cx| {
                range
                    .map(|ix| this.render_tree_row(this.rows[ix], cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.tree_scroll)
        .flex_1()
        .min_h_0()
        .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
            if this.state.replay_at().is_none() {
                let up = f32::from(event.delta.pixel_delta(px(ROW_HEIGHT)).y) > 0.;
                let at_end = this.tree_scroll.is_scrolled_to_end().unwrap_or(true);
                this.state.set_tail(!up && at_end);
                cx.notify();
            }
        }))
        .into_any_element()
    }

    fn render_tree_row(&self, row: TreeRow, cx: &mut Context<Self>) -> AnyElement {
        let node = self.model.node(row.id);
        let selected = self.state.selected() == Some(node.key());
        let filtering = self.state.filter().is_active();
        let open = self.state.is_open(&self.model, row.id, filtering);
        if node.is_turn() {
            return self.render_turn_row(row, node, open, selected, cx);
        }
        let id = row.id;
        let key = node.key().clone();
        let label = label(node, self.summaries(), &self.model);
        let look = look(node);
        let turn = self.model.node(row.turn);
        let t0 = turn.start().unwrap_or(self.model.end());
        let span = (turn.end().unwrap_or(self.model.end()) - t0).max(1);
        let start = node.start().unwrap_or(t0);
        let until = node.end().unwrap_or(self.model.end());
        let timed = !matches!(node.kind(), NodeKind::Event(_) | NodeKind::Note(_));
        let dur = timed.then(|| (until - start).max(0));
        let in_agent = node.depth() > 1
            && node
                .parent()
                .is_some_and(|p| self.model.node(p).is_subagent());
        let err = look.tone == Tone::Err;
        let toks = row_tokens(node);
        let has_children = !node.children().is_empty();
        #[expect(
            clippy::cast_precision_loss,
            reason = "millisecond offsets are far below 2^52"
        )]
        let (left, width) = (
            (start - t0) as f32 / span as f32,
            ((until - start) as f32 / span as f32).max(0.),
        );
        let text_color = if err {
            theme::RED_SOFT
        } else if label.quiet || matches!(node.kind(), NodeKind::Model(_)) {
            theme::QUIET
        } else if label.ai {
            theme::FG
        } else {
            theme::TEXT_2
        };
        let quiet = label.quiet || matches!(node.kind(), NodeKind::Model(_));
        let raw = !quiet && !label.ai;
        #[expect(clippy::cast_precision_loss, reason = "tree depth is small")]
        let indent = 14. + (node.depth().saturating_sub(1)) as f32 * 20.;
        let toggle_key = key.clone();

        div()
            .id(("row", row.id_index()))
            // Rows are laid out one by one; without a width each fits its own text
            // and the columns stop lining up.
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .h(px(ROW_HEIGHT))
            .pl(px(indent))
            .pr(px(14.))
            .border_b_1()
            .border_color(color(theme::ROW_LINE))
            .cursor_pointer()
            .when(err, |d| d.bg(color(theme::RED_BG)))
            .when(!err && in_agent, |d| d.bg(color(theme::SUB_BG)))
            .when(self.is_fresh(node.key()), |d| d.bg(color(theme::FRESH)))
            .when(selected, |d| {
                d.bg(color(theme::SEL))
                    .border_l_2()
                    .border_color(color(theme::BLUE))
            })
            .hover(|s| s.bg(color(theme::HOVER)))
            .on_click(
                cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                    if event.click_count() > 1 && has_children {
                        this.state.toggle(&this.model, id);
                        this.refresh();
                    }
                    this.select(key.clone(), false, cx);
                }),
            )
            .child(
                div()
                    .w(px(16.))
                    .flex_none()
                    .text_size(px(theme::FS_SMALL))
                    .text_color(tone_icon(look.icon_tone))
                    .child(look.icon),
            )
            .child(
                div()
                    .w(px(46.))
                    .flex_none()
                    .font_family(MONO)
                    .text_size(px(theme::FS_TINY))
                    .text_color(tone_text(look.tone))
                    .truncate()
                    .child(look.badge),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .when(has_children, |d| {
                        d.child(
                            div()
                                .id(("caret", row.id_index()))
                                .flex_none()
                                .mr_1()
                                .text_color(color(theme::PURPLE))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(id) = this.model.find(&toggle_key) {
                                        this.state.toggle(&this.model, id);
                                        this.refresh();
                                        cx.notify();
                                    }
                                }))
                                .child(if open { "▾" } else { "▸" }),
                        )
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(color(text_color))
                            .when(quiet, gpui_kit::Styled::italic)
                            .when(raw, |d| d.font_family(MONO).text_size(px(11.)))
                            .child(one_line(&label.text)),
                    ),
            )
            .child(numeric(56., theme::DIM).child(if toks > 0 {
                tokens(toks)
            } else {
                String::new()
            }))
            .child(
                numeric(
                    64.,
                    match look.tone {
                        Tone::Err => theme::RED_TEXT,
                        Tone::Run => theme::BLUE_TEXT,
                        _ => theme::BTN,
                    },
                )
                .when(dur.is_some_and(|d| d >= LONG_MS), |d| {
                    d.font_weight(FontWeight::SEMIBOLD)
                })
                .child(match dur {
                    Some(ms) => format!(
                        "{}{}",
                        duration(ms),
                        if node.end().is_none() { "…" } else { "" }
                    ),
                    None => String::new(),
                }),
            )
            .child(
                div()
                    .relative()
                    .w(px(160.))
                    .flex_none()
                    .h(px(7.))
                    .bg(color(theme::ROW_LINE))
                    .rounded(px(2.))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(left))
                            .w(relative(width))
                            .min_w(px(2.))
                            .rounded(px(1.))
                            .bg(tone_bar(look.tone)),
                    ),
            )
            .test_support()
            .into_any_element()
    }

    fn render_turn_row(
        &self,
        row: TreeRow,
        node: &Node,
        open: bool,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = label(node, self.summaries(), &self.model);
        debug_assert_eq!(label.chip_class, ChipClass::Turn);
        let number = self
            .model
            .turns()
            .iter()
            .filter(|&&t| {
                matches!(
                    self.model.node(t).kind(),
                    NodeKind::Turn {
                        prompt: Some(_),
                        ..
                    }
                )
            })
            .position(|&t| t == row.id)
            .map_or_else(|| "·".to_owned(), |i| (i + 1).to_string());
        let outcome = label
            .sub
            .as_deref()
            .and_then(|sub| sub.strip_prefix("→ "))
            .and_then(|sub| sub.split_once('\n'))
            .map(|(outcome, _)| one_line(outcome));
        let end = self.model.end();
        let start = node.start().unwrap_or(end);
        let dur = (node.end().unwrap_or(end) - start).max(0);
        let longest = self
            .model
            .turns()
            .iter()
            .map(|&t| {
                let t = self.model.node(t);
                t.end().unwrap_or(end) - t.start().unwrap_or(end)
            })
            .max()
            .unwrap_or(1)
            .max(1);
        #[expect(
            clippy::cast_precision_loss,
            reason = "millisecond spans are far below 2^52"
        )]
        let bar = (dur as f32 / longest as f32).max(0.01);
        let id = row.id;
        let key = node.key().clone();

        div()
            .id(("turn", row.id_index()))
            // Rows are laid out one by one; without a width each fits its own text
            // and the columns stop lining up.
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .h(px(ROW_HEIGHT))
            .px(px(14.))
            .border_b_1()
            .border_color(color(if open {
                theme::BLUE_LINE
            } else {
                theme::SOFT_LINE
            }))
            .cursor_pointer()
            .when(open, |d| d.bg(color(theme::TURN_OPEN)))
            .when(selected, |d| {
                d.bg(color(theme::SEL))
                    .border_l_2()
                    .border_color(color(theme::BLUE))
            })
            .hover(|s| s.bg(color(theme::HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.state.toggle(&this.model, id);
                this.refresh();
                this.select(key.clone(), false, cx);
            }))
            .child(
                div()
                    .w(px(30.))
                    .flex_none()
                    .font_family(MONO)
                    .text_size(px(theme::FS_TINY))
                    .whitespace_nowrap()
                    .text_color(color(if open { theme::BLUE_TEXT } else { theme::DIM }))
                    .child(format!("{number} {}", if open { "▾" } else { "▸" })),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(
                        div()
                            .flex_shrink(1.)
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(color(theme::BRIGHT))
                            .child(one_line(&label.text)),
                    )
                    .when_some(outcome, |d, outcome| {
                        d.child(
                            div()
                                .flex_1()
                                .min_w(px(40.))
                                .truncate()
                                .text_color(color(theme::MUTED))
                                .child(format!("→ {outcome}")),
                        )
                    })
                    .when(node.running(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_none()
                                .items_center()
                                .gap_1()
                                .text_size(px(theme::FS_SMALL))
                                .text_color(color(theme::BLUE_TEXT))
                                .child(div().size(px(6.)).rounded(px(3.)).bg(color(theme::BLUE)))
                                .child("running"),
                        )
                    })
                    .when(node.errors() > 0, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .text_size(px(theme::FS_SMALL))
                                .text_color(color(theme::RED_TEXT))
                                .child(crate::present::plural(node.errors(), "error")),
                        )
                    }),
            )
            .child(numeric(56., theme::DIM).child({
                let out = node.totals().output_tokens;
                if out > 0 { tokens(out) } else { String::new() }
            }))
            .child(
                numeric(64., theme::BTN)
                    .when(dur >= LONG_MS, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .child(format!(
                        "{}{}",
                        duration(dur),
                        if node.running() { "…" } else { "" }
                    )),
            )
            .child(
                div().w(px(160.)).flex_none().child(
                    div()
                        .h(px(6.))
                        .w(relative(bar))
                        .min_w(px(2.))
                        .rounded(px(2.))
                        .bg(color(if open {
                            theme::BLUE_BAR
                        } else {
                            theme::TURN_BAR
                        })),
                ),
            )
            .test_support()
            .into_any_element()
    }
}

impl TreeRow {
    /// A stable element id for the row.
    fn id_index(self) -> usize {
        self.id.index()
    }
}

/// A right-aligned mono number column.
pub(super) fn numeric(width: f32, text: u32) -> gpui_kit::Div {
    div()
        .w(px(width))
        .flex_none()
        .flex()
        .justify_end()
        .font_family(MONO)
        .text_size(px(theme::FS_SMALL))
        .whitespace_nowrap()
        .text_color(color(text))
}

/// The italic placeholder shown instead of rows.
pub(super) fn empty(text: &'static str) -> gpui_kit::Div {
    div()
        .flex_1()
        .px(px(14.))
        .py_4()
        .italic()
        .text_color(color(theme::DIM))
        .child(text)
}
