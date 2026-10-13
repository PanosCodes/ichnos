//! The agent inspector: what one agent of the graph did, beside the graph.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use ichnos_model::labels::{duration, tokens};

use super::Viewer;
use super::graph::{agent_badge, facts};
use super::inspector::{body, button, heading, key_values};
use crate::theme::{self, color};

impl Viewer {
    pub(super) fn render_agent_inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let graph = self.agent_graph();
        let entry = graph
            .find(self.state.agent())
            .unwrap_or_else(|| graph.main());
        let facts = facts(entry, &self.model, self);
        let filter = if facts.main {
            "agent:main".to_owned()
        } else {
            format!("agent:{}", entry.agent_type().unwrap_or("subagent"))
        };
        let status_color = if entry.running() {
            theme::BLUE_TEXT
        } else if entry.errors() > 0 {
            theme::RED_TEXT
        } else {
            theme::GREEN
        };
        let returned = if facts.returned.is_empty() {
            if entry.running() {
                "still running…"
            } else {
                "—"
            }
            .to_owned()
        } else {
            facts.returned.clone()
        };
        let prompt = if facts.full_prompt.is_empty() {
            facts.prompt.clone()
        } else {
            facts.full_prompt.clone()
        };
        div()
            .flex()
            .flex_col()
            .w(px(self.state.panel().width()))
            .flex_none()
            .min_h_0()
            .bg(color(theme::PANEL))
            .child(
                body("agent-body")
                    .pt(px(14.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(agent_badge(facts.main))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(theme::FS_BASE))
                                    .text_color(color(theme::BRIGHT))
                                    .child(facts.name.clone()),
                            )
                            .child(
                                div()
                                    .ml_auto()
                                    .text_size(px(theme::FS_SMALL))
                                    .text_color(color(status_color))
                                    .child(facts.status.clone()),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(3.))
                            .text_size(px(theme::FS_SMALL))
                            .text_color(color(theme::DIM))
                            .child(format!("spawned {}", facts.spawned)),
                    )
                    .child(heading("Prompt"))
                    .child(super::inspector::block(
                        crate::present::SectionTone::Prose,
                        &prompt,
                        cx,
                    ))
                    .child(heading("Returned"))
                    .child(super::inspector::block(
                        crate::present::SectionTone::Prose,
                        &returned,
                        cx,
                    ))
                    .child(div().mt_4().child(key_values(&[
                        ("wall", facts.dur.map_or_else(|| "—".to_owned(), duration)),
                        ("model", duration(entry.model_ms())),
                        ("tools", duration(facts.tool_ms)),
                        ("calls", facts.calls.to_string()),
                        ("errors", entry.errors().to_string()),
                        ("out", tokens(entry.output_tokens())),
                    ])))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_1p5()
                            .mt_4()
                            .text_size(px(theme::FS_SMALL))
                            .when_some(facts.key.clone(), |d, key| {
                                d.child(
                                    button("open-in-tree", "Open in tree →", true, true).on_click(
                                        cx.listener(move |this, _, _, cx| this.reveal(&key, cx)),
                                    ),
                                )
                            })
                            .child(
                                button("filter-agent", "Filter to this agent", false, true)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.set_query(&filter, window, cx);
                                        this.set_view(crate::view_state::View::Tree, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}
