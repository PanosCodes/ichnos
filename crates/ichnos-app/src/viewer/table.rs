//! The table: every entry in time order, subagent start and stop events included.

use accesskit::Role;
use std::ops::Range;

use gpui_kit::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, uniform_list,
};
use ichnos_core::event::Status;
use ichnos_model::labels::{clock, duration, skill_name, summarize};

use super::tree::empty;
use super::{ROW_HEIGHT, Viewer, one_line};
use crate::theme::{self, MONO, color};

/// Column widths: time, agent, tool or event, status, duration; the summary takes the rest.
const COLUMNS: [f32; 5] = [80., 110., 140., 80., 96.];

/// Columns dropped (agent, status) when the main pane is narrower than [`COMPACT_BELOW`].
const OPTIONAL: [usize; 2] = [1, 3];

/// Main pane width, in pixels, below which the table drops its optional columns.
const COMPACT_BELOW: f32 = 760.;

impl Viewer {
    pub(super) fn render_table(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let compact = self
            .state
            .panel()
            .main_width(f32::from(window.viewport_size().width))
            < COMPACT_BELOW;
        if self.table.is_empty() {
            return empty("No entries.").into_any_element();
        }
        let header = [
            "time",
            "agent",
            "tool / event",
            "status",
            "duration",
            "summary",
        ];
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .h(px(28.))
                    .items_center()
                    .border_b_1()
                    .border_color(color(theme::LINE))
                    .text_size(px(theme::FS_SMALL))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(color(theme::DIM))
                    .children(
                        header
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| !(compact && OPTIONAL.contains(i)))
                            .map(|(i, name)| cell(i).child(name.to_uppercase())),
                    ),
            )
            .child(
                uniform_list(
                    "table",
                    self.table.len(),
                    cx.processor(move |this, range: Range<usize>, _window, cx| {
                        range
                            .map(|ix| this.render_table_row(ix, compact, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.table_scroll)
                .flex_1()
                .min_h_0(),
            )
            .into_any_element()
    }

    fn render_table_row(&self, ix: usize, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let row = &self.table[ix];
        let call = &row.call;
        let key = call.key().clone();
        let ai = self.summaries().get(&key);
        let input = if call.input().is_null() {
            call.prompt()
        } else {
            call.input()
        };
        let what = ai.cloned().unwrap_or_else(|| summarize(input));
        let failed = call.status() == Some(Status::Error);
        let selected = self.state.selected() == Some(&key);
        let dur = call.duration_ms().or_else(|| {
            (call.status() == Some(Status::Started)).then(|| self.model.end() - call.start())
        });
        let row_id = call.first().row_id();
        let row = div()
            .id(("table-row", ix))
            .role(Role::Button)
            .aria_label(format!(
                "{}: {}",
                call.tool_name().unwrap_or(call.name()),
                one_line(&what)
            ))
            // Rows are laid out one by one; without a width each fits its own text
            // and the columns stop lining up.
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .h(px(ROW_HEIGHT))
            .border_b_1()
            .border_color(color(theme::ROW_LINE))
            .cursor_pointer()
            .when(failed, |d| d.text_color(color(theme::RED_SOFT)))
            .when(selected, |d| {
                d.bg(color(theme::SEL))
                    .border_b_0()
                    .border_l_2()
                    .border_color(color(theme::BLUE))
            })
            .hover(|s| s.bg(color(theme::HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| this.select(key.clone(), false, cx)))
            .child(mono(cell(0)).child(clock(call.start())))
            .when(!compact, |d| {
                d.child(cell(1).child(call.agent_type().unwrap_or(call.agent_id()).to_owned()))
            })
            .child(cell(2).child(call.tool_name().unwrap_or(call.name()).to_owned()))
            .when(!compact, |d| {
                d.child(cell(3).child(call.status().map(Status::as_str).unwrap_or_default()))
            })
            .child(mono(cell(4)).child(dur.map(duration).unwrap_or_default()))
            .child(
                cell(5)
                    .when(ai.is_none(), mono)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .overflow_hidden()
                    .child(div().flex_1().min_w_0().truncate().child(one_line(&what)))
                    .when_some(skill_name(call), |d, name| d.child(Viewer::skill_tag(name))),
            )
            .test_support();
        Viewer::with_row_menu(row, call.key().clone(), Some(row_id), cx)
    }
}

/// Column `i`'s cell.
fn cell(i: usize) -> gpui_kit::Div {
    let cell = div().px(px(10.)).truncate().whitespace_nowrap();
    match COLUMNS.get(i) {
        Some(&width) => cell.w(px(width)).flex_none(),
        None => cell.flex_1().min_w_0(),
    }
}

/// A cell in mono.
fn mono(cell: gpui_kit::Div) -> gpui_kit::Div {
    cell.font_family(MONO)
        .text_size(px(theme::FS_SMALL))
        .text_color(color(theme::MUTED))
}
