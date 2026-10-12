//! The inspector: the selected row's Call tab, and the State, Files and Tokens tabs.

use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use accesskit::Role;
use gpui_kit::component::menu::{ContextMenu, ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, StyleRefinement,
    Styled as _, Window, div, px, relative,
};
use ichnos_model::call::Key;
use ichnos_model::labels::{clock, duration, label, tokens};
use ichnos_model::model::{FileTouch, NodeId, NodeKind, StateAt};

use super::{FOR_AGENT, Viewer, one_line};
use crate::present::{SectionTone, Tone, look, pretty, sections, status_text};
use crate::theme::{self, MONO, color, tone_text};
use crate::view_state::{InspectorTab, View};
use gpui_kit::TestSupportExt as _;

/// Where the Todo list's status icons come from.
fn todo_icon(status: &str) -> &'static str {
    match status {
        "completed" => "☑",
        "in_progress" => "◐",
        _ => "☐",
    }
}

impl Viewer {
    /// The node shown in the inspector, and the rows before and after it.
    fn inspected(&self) -> (Option<NodeId>, Option<Key>, Option<Key>) {
        let Some(key) = self.state.selected() else {
            return (None, None, None);
        };
        let order = self.order();
        let at = order.iter().position(|k| k == key);
        let prev = at
            .and_then(|at| at.checked_sub(1))
            .map(|at| order[at].clone());
        let next = at.and_then(|at| order.get(at + 1)).cloned();
        (self.model.find(key), prev, next)
    }

    pub(super) fn render_inspector(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (node, prev, next) = self.inspected();
        let end = self.model.end();
        let at = self
            .state
            .replay_at()
            .unwrap_or_else(|| node.map_or(end, |id| self.model.node(id).end().unwrap_or(end)));
        let state = self.model.state_at(at);
        let tab = self.state.tab();
        let is_turn = node.is_some_and(|id| self.model.node(id).is_turn());
        let files = if state.changed.is_empty() {
            "Files".to_owned()
        } else {
            format!("Files {}", state.changed.len())
        };
        let tabs = [
            (
                InspectorTab::Call,
                if is_turn {
                    "Turn".to_owned()
                } else {
                    "Call".to_owned()
                },
            ),
            (InspectorTab::State, "State".to_owned()),
            (InspectorTab::Files, files),
            (InspectorTab::Tokens, "Tokens".to_owned()),
        ];
        let why = if self.state.replay_at().is_some() {
            "replay cursor"
        } else if node.is_some() {
            "end of the selected row"
        } else {
            "latest"
        };
        let body = match tab {
            InspectorTab::Call => self.render_call_tab(node, prev, next, cx),
            InspectorTab::State => self.render_state_tab(&state, cx),
            InspectorTab::Files => Self::render_files_tab(&state, cx),
            InspectorTab::Tokens => self.render_tokens_tab(cx),
        };
        // Every tab but Call describes the whole session, not the row: say at which moment.
        let t0 = self.bounds.map_or(0, |b| b.t0);
        let scope = match tab {
            InspectorTab::Call => None,
            InspectorTab::State | InspectorTab::Files => Some(format!(
                "Session at {} · +{} · {why}",
                clock(at),
                duration((at - t0).max(0))
            )),
            InspectorTab::Tokens => Some("Whole session".to_owned()),
        };
        div()
            .flex()
            .flex_col()
            .w(px(self.state.panel().width()))
            .flex_none()
            .min_h_0()
            .bg(color(theme::PANEL))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_end()
                    .gap_4()
                    .h(px(36.))
                    .px_4()
                    .border_b_1()
                    .border_color(color(theme::LINE))
                    .text_color(color(theme::MUTED))
                    .children(tabs.into_iter().map(|(id, name)| {
                        let on = id == tab;
                        div()
                            .id(SharedString::from(format!("tab-{id:?}")))
                            .role(Role::Tab)
                            .aria_label(name.clone())
                            .aria_selected(on)
                            .pb(px(9.))
                            .cursor_pointer()
                            .when(on, |d| {
                                d.text_color(color(theme::WHITE))
                                    .border_b_2()
                                    .border_color(color(theme::BLUE))
                            })
                            .when(!on, |d| d.hover(|s| s.text_color(color(theme::FG))))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.state.set_tab(id);
                                cx.notify();
                            }))
                            .child(name)
                    })),
            )
            .when_some(scope, |d, scope| {
                d.child(
                    div()
                        .id("inspector-scope")
                        .flex_none()
                        .px_4()
                        .py(px(6.))
                        .border_b_1()
                        .border_color(color(theme::SOFT_LINE))
                        .font_family(MONO)
                        .text_size(px(theme::FS_SMALL))
                        .text_color(color(theme::MUTED))
                        .truncate()
                        .child(scope),
                )
            })
            .child(body)
    }

    fn render_call_tab(
        &self,
        node: Option<NodeId>,
        prev: Option<Key>,
        next: Option<Key>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(id) = node else {
            return body("call-empty")
                .child(
                    div()
                        .italic()
                        .text_color(color(theme::DIM))
                        .child("Select a row to inspect it. j/k move through rows."),
                )
                .into_any_element();
        };
        let node = self.model.node(id);
        let summaries = self.summaries();
        let text = label(node, summaries, &self.model).text;
        let look = look(node);
        let end = self.model.end();
        let t0 = self.bounds.map_or(0, |b| b.t0);
        let start = node.start().unwrap_or(t0);
        let until = node.end().unwrap_or(end);
        let since = format!("+{}", duration((start - t0).max(0)));
        let when = if matches!(node.kind(), NodeKind::Event(_) | NodeKind::Note(_)) {
            since
        } else {
            format!("{since} · {}", duration((until - start).max(0)))
        };
        let raw = pretty(&serde_json::Value::Array(node.raw_data()));
        let around = |key: &Key| {
            self.model.find(key).map_or_else(
                || key.to_string(),
                |id| one_line(&label(self.model.node(id), summaries, &self.model).text),
            )
        };
        let is_turn = node.is_turn();
        let replay_from = start;

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex_none()
                    .px_4()
                    .pt(px(14.))
                    .pb_3()
                    .border_b_1()
                    .border_color(color(theme::LINE))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(badge(look.tone, look.badge.clone()))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(tone_text(look.tone))
                                    .child(status_text(node)),
                            )
                            .child(
                                div()
                                    .ml_auto()
                                    .font_family(MONO)
                                    .text_size(px(theme::FS_SMALL))
                                    .text_color(color(theme::MUTED))
                                    .whitespace_nowrap()
                                    .child(when),
                            )
                            .when_some(node.row_id(), |d, row_id| {
                                d.child(
                                    link(
                                        FOR_AGENT.into(),
                                        if self.is_copied(FOR_AGENT) {
                                            "copied"
                                        } else {
                                            "copy for agent"
                                        },
                                    )
                                    .whitespace_nowrap()
                                    .text_size(px(theme::FS_SMALL))
                                    .text_color(color(theme::DIM))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.copy_for_agent(row_id, cx);
                                    }))
                                    .test_support(),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id("call-title")
                            .mt_2()
                            .line_clamp(2)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(color(theme::BRIGHT))
                            .child(text),
                    ),
            )
            .child(
                body("call-body")
                    .children(
                        sections(node, &self.model, summaries)
                            .into_iter()
                            .enumerate()
                            .map(|(i, section)| {
                                let copy_id = SharedString::from(format!("copy-{i}"));
                                let raw_id = SharedString::from("copy-raw");
                                let copied_text = self.is_copied(&copy_id);
                                let copied_raw = self.is_copied(&raw_id);
                                let section_body = section.body.clone();
                                let raw = raw.clone();
                                div()
                                    .child(
                                        heading(section.title)
                                            .when(i == 0, gpui_kit::Styled::mt_0)
                                            .child(
                                                div()
                                                    .ml_auto()
                                                    .flex()
                                                    .flex_row()
                                                    .gap_2()
                                                    .text_size(px(theme::FS_SMALL))
                                                    .text_color(color(theme::DIM))
                                                    .when(i == 0, |d| {
                                                        d.child(
                                                            link(
                                                                raw_id.clone(),
                                                                if copied_raw {
                                                                    "copied"
                                                                } else {
                                                                    "raw"
                                                                },
                                                            )
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    this.copy(
                                                                        raw_id.clone(),
                                                                        raw.clone(),
                                                                        cx,
                                                                    );
                                                                },
                                                            )),
                                                        )
                                                    })
                                                    .child(
                                                        link(
                                                            copy_id.clone(),
                                                            if copied_text {
                                                                "copied"
                                                            } else {
                                                                "copy"
                                                            },
                                                        )
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.copy(
                                                                    copy_id.clone(),
                                                                    section_body.clone(),
                                                                    cx,
                                                                );
                                                            },
                                                        )),
                                                    ),
                                            ),
                                    )
                                    .child(block(section.tone, &section.body, cx))
                            }),
                    )
                    .when(prev.is_some() || next.is_some(), |d| {
                        d.child(heading(if is_turn {
                            "Around this turn"
                        } else {
                            "Around this call"
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .text_size(px(theme::FS_SMALL))
                                .children(prev.clone().map(|key| {
                                    let text = around(&key);
                                    around_row("before", text).on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.select(key.clone(), true, cx);
                                        },
                                    ))
                                }))
                                .children(next.clone().map(|key| {
                                    let text = around(&key);
                                    around_row("after", text).on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.select(key.clone(), true, cx);
                                        },
                                    ))
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .gap_1p5()
                    .px_4()
                    .py(px(10.))
                    .border_t_1()
                    .border_color(color(theme::LINE))
                    .text_size(px(theme::FS_SMALL))
                    .child(
                        button("prev-row", "◀ prev", false, prev.is_some()).when_some(
                            prev,
                            |d, key| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.select(key.clone(), true, cx);
                                }))
                            },
                        ),
                    )
                    .child(
                        button("next-row", "next ▶", false, next.is_some()).when_some(
                            next,
                            |d, key| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.select(key.clone(), true, cx);
                                }))
                            },
                        ),
                    )
                    .child(
                        button("ask-row", "Ask about this", true, true)
                            .ml_auto()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_ask(true, window, cx);
                            })),
                    )
                    .child(
                        button("replay-row", "Replay from here", true, true).on_click(
                            cx.listener(move |this, _, _, cx| this.seek(replay_from, cx)),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_state_tab(&self, state: &StateAt, cx: &mut Context<Self>) -> AnyElement {
        let summaries = self.summaries();
        body("state-body")
            .child(heading("At this moment"))
            .child(key_values(&[
                ("turns", state.turns.to_string()),
                ("calls", state.calls.to_string()),
                ("errors", state.errors.to_string()),
                ("out", tokens(state.output_tokens)),
            ]))
            .child(heading("Running"))
            .child(if state.running.is_empty() {
                dim("nothing").into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .children(state.running.iter().map(|&id| {
                        let node = self.model.node(id);
                        let look = look(node);
                        let key = node.key().clone();
                        div()
                            .id(("running", id.index()))
                            .role(Role::Button)
                            .aria_label("Reveal running call in tree")
                            .flex()
                            .flex_row()
                            .gap_2()
                            .min_w_0()
                            .cursor_pointer()
                            .hover(|s| s.text_color(color(theme::BRIGHT)))
                            .on_click(cx.listener(move |this, _, _, cx| this.reveal(&key, cx)))
                            .child(badge(look.tone, look.badge))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(one_line(&label(node, summaries, &self.model).text)),
                            )
                    }))
                    .into_any_element()
            })
            .child(heading("Todo list"))
            .child(match &state.todos {
                None => dim("no todos yet").into_any_element(),
                Some(todos) => div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .children(todos.iter().map(|todo| {
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(todo_icon(&todo.status))
                            .child(div().min_w_0().child(todo.content.clone()))
                    }))
                    .into_any_element(),
            })
            .child(heading("Skills loaded"))
            .child(if state.skills.is_empty() {
                dim("none").into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .children(state.skills.iter().map(|skill| div().child(skill.clone())))
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_files_tab(state: &StateAt, cx: &mut Context<Self>) -> AnyElement {
        let list = |files: &[FileTouch], changed: bool, cx: &mut Context<Self>| -> AnyElement {
            if files.is_empty() {
                return dim("none").into_any_element();
            }
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .children(files.iter().enumerate().map(|(i, file)| {
                    let name = file
                        .path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&file.path)
                        .to_owned();
                    let count = if changed {
                        format!("{}×{}", file.times, if file.failed { " ✕" } else { "" })
                    } else {
                        format!("{}×", file.times)
                    };
                    div()
                        .id(ElementId::Name(format!("file-{changed}-{i}").into()))
                        .role(Role::Button)
                        .aria_label(format!("Filter to {name}"))
                        .flex()
                        .flex_row()
                        .gap_2()
                        .min_w_0()
                        .cursor_pointer()
                        .hover(|s| s.text_color(color(theme::BRIGHT)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_query(&format!("file:{name}"), window, cx);
                            this.set_view(View::Tree, cx);
                        }))
                        .child(
                            div()
                                .min_w_0()
                                .text_ellipsis_start()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(file.path.clone()),
                        )
                        .child(
                            div()
                                .ml_auto()
                                .flex_none()
                                .font_family(MONO)
                                .text_size(px(theme::FS_SMALL))
                                .text_color(color(theme::DIM))
                                .child(count),
                        )
                }))
                .into_any_element()
        };
        let changed = list(&state.changed, true, cx);
        let read = list(&state.read, false, cx);
        body("files-body")
            .child(heading(format!("Changed ({})", state.changed.len())).mt_0())
            .child(changed)
            .child(heading(format!("Read ({})", state.read.len())))
            .child(read)
            .child(note(
                "Click a file to filter the tree to it. Files changed through Bash (sed, scripts) are not tracked.",
            ))
            .into_any_element()
    }

    fn render_tokens_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let summaries = self.summaries();
        let turns: Vec<NodeId> = self
            .model
            .turns()
            .iter()
            .copied()
            .filter(|&t| {
                let node = self.model.node(t);
                matches!(
                    node.kind(),
                    NodeKind::Turn {
                        prompt: Some(_),
                        ..
                    }
                ) || node.totals().output_tokens > 0
            })
            .collect();
        let max_turn = turns
            .iter()
            .map(|&t| self.model.node(t).totals().output_tokens)
            .max()
            .unwrap_or(1)
            .max(1);
        let mut steps: Vec<(NodeId, i64)> = self
            .model
            .iter()
            .filter_map(|(id, node)| node.model_step().map(|m| (id, m.step().output_tokens())))
            .collect();
        steps.sort_by_key(|&(_, out)| std::cmp::Reverse(out));
        steps.truncate(8);
        let max_step = steps.iter().map(|&(_, out)| out).max().unwrap_or(1).max(1);

        let bar = |id: NodeId, name: String, value: i64, max: i64, cx: &mut Context<Self>| {
            let key = self.model.node(id).key().clone();
            #[expect(
                clippy::cast_precision_loss,
                reason = "token counts are far below 2^52"
            )]
            let share = value as f32 / max as f32;
            div()
                .id(("tokens", id.index()))
                .role(Role::Button)
                .aria_label("Reveal call in tree")
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .h(px(22.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.reveal(&key, cx)))
                .child(div().flex_1().min_w_0().truncate().child(name))
                .child(
                    div()
                        .w(px(120.))
                        .flex_none()
                        .h(px(5.))
                        .rounded(px(2.))
                        .bg(color(theme::SOFT_LINE))
                        .child(
                            div()
                                .h(px(5.))
                                .w(relative(share))
                                .min_w(px(2.))
                                .rounded(px(2.))
                                .bg(color(theme::BLUE_BAR)),
                        ),
                )
                .child(super::tree::numeric(48., theme::DIM).child(tokens(value)))
        };

        let mut turn_bars = Vec::new();
        for (i, &t) in turns.iter().enumerate() {
            let node = self.model.node(t);
            let name = if matches!(
                node.kind(),
                NodeKind::Turn {
                    prompt: Some(_),
                    ..
                }
            ) {
                format!(
                    "{} · {}",
                    i + 1,
                    one_line(&label(node, summaries, &self.model).text)
                )
            } else {
                "before the first prompt".to_owned()
            };
            turn_bars.push(bar(t, name, node.totals().output_tokens, max_turn, cx));
        }
        let mut step_bars = Vec::new();
        for &(id, out) in &steps {
            let name = one_line(&label(self.model.node(id), summaries, &self.model).text);
            step_bars.push(bar(id, name, out, max_step, cx));
        }

        body("tokens-body")
            .child(heading("Output tokens per turn").mt_0())
            .children(turn_bars)
            .child(heading("Largest model steps"))
            .when(step_bars.is_empty(), |d| d.child(dim("no model steps yet")))
            .children(step_bars)
            .child(note(
                "Counts come from the usage the agent reported. Input tokens include cache reads.",
            ))
            .into_any_element()
    }
}

/// The scrolling body of an inspector tab.
pub(super) fn body(id: &'static str) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px_4()
        .py_3()
}

/// A small uppercase section heading.
pub(super) fn heading(title: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .mt(px(14.))
        .mb(px(6.))
        .child(
            div()
                .text_size(px(theme::FS_SMALL))
                .text_color(color(theme::DIM))
                .child(title.into().to_uppercase()),
        )
}

/// A prose or code block whose text can be selected with the mouse.
///
/// The text is shown as a fenced code block, so Markdown in it is left alone.
pub(super) fn block(tone: SectionTone, text: &str, cx: &Context<Viewer>) -> impl IntoElement {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    let id = SharedString::from(format!("block-{:x}", hasher.finish()));
    let longest_run = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest_run.max(2) + 1);
    let body = TextView::markdown(id.clone(), format!("{fence}\n{text}\n{fence}"))
        .selectable(true)
        .style(
            TextViewStyle::default().code_block(
                StyleRefinement::default()
                    .bg(gpui_kit::transparent_black())
                    .p_0()
                    .font_family(MONO)
                    .text_size(px(theme::FS_SMALL)),
            ),
        );
    let block = div()
        .id(SharedString::from(format!("{id}-content")))
        .px(px(10.))
        .py_2()
        .rounded(px(theme::R_CONTROL))
        .border_1()
        .bg(color(theme::FIELD))
        .border_color(color(theme::LINE));
    let block = match tone {
        SectionTone::Prose => block.text_color(color(theme::FG)),
        SectionTone::Code => block.text_color(color(theme::TEXT_2)),
        SectionTone::Err => block
            .bg(color(theme::RED_BG))
            .border_color(color(theme::RED_LINE))
            .text_color(color(theme::RED_SOFT)),
    }
    .child(body);
    text_area(id, text.to_owned(), block, cx)
}

/// A black text area whose context menu copies its complete text.
pub(super) fn text_area(
    id: SharedString,
    text: String,
    area: gpui_kit::Stateful<Div>,
    cx: &Context<Viewer>,
) -> ContextMenu<gpui_kit::Stateful<Div>> {
    let viewer = cx.entity().downgrade();
    div()
        .id(id.clone())
        .child(area)
        .context_menu(move |menu, _, _| {
            let (viewer, id, text) = (viewer.clone(), id.clone(), text.clone());
            menu.item(PopupMenuItem::new("Copy").on_click(move |_, _, cx| {
                viewer
                    .update(cx, |this, cx| this.copy(id.clone(), text.clone(), cx))
                    .ok();
            }))
        })
}

/// A mono badge in `tone`.
pub(super) fn badge(tone: Tone, text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .font_family(MONO)
        .text_size(px(theme::FS_SMALL))
        .border_1()
        .border_color(color(theme::LINE_2))
        .rounded(px(theme::R_TAG))
        .px(px(5.))
        .whitespace_nowrap()
        .text_color(tone_text(tone))
        .child(text.into())
}

/// A clickable dim text link.
fn link(id: SharedString, text: &'static str) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .role(Role::Link)
        .aria_label(text)
        .cursor_pointer()
        .hover(|s| s.text_color(color(theme::FG)))
        .child(text)
}

/// A bordered button; `accent` makes it blue, `enabled: false` dims it.
pub(super) fn button(
    id: &'static str,
    text: &'static str,
    accent: bool,
    enabled: bool,
) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(text)
        .px(px(9.))
        .py_1()
        .border_1()
        .border_color(color(theme::LINE_2))
        .rounded(px(theme::R_CONTROL))
        .whitespace_nowrap()
        .text_color(color(if accent {
            theme::BLUE_TEXT
        } else {
            theme::TEXT_2
        }))
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(|s| s.text_color(color(theme::BRIGHT)))
        })
        .when(!enabled, |d| d.opacity(0.4))
        .child(text)
}

/// A "before"/"after" neighbor row.
fn around_row(which: &'static str, text: String) -> gpui_kit::Stateful<Div> {
    div()
        .id(which)
        .role(Role::Button)
        .aria_label(format!("{which}: {text}"))
        .flex()
        .flex_row()
        .gap_2()
        .min_w_0()
        .cursor_pointer()
        .text_color(color(theme::MUTED))
        .hover(|s| s.text_color(color(theme::FG)))
        .child(
            div()
                .w(px(52.))
                .flex_none()
                .whitespace_nowrap()
                .font_family(MONO)
                .text_size(px(theme::FS_SMALL))
                .child(which),
        )
        .child(div().min_w_0().truncate().italic().child(text))
}

/// Label/value pairs in two mono columns.
pub(super) fn key_values(pairs: &[(&'static str, String)]) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .font_family(MONO)
        .text_size(px(theme::FS_SMALL))
        .children(pairs.iter().map(|(name, value)| {
            div()
                .flex()
                .flex_row()
                .gap(px(14.))
                .child(div().w(px(56.)).text_color(color(theme::DIM)).child(*name))
                .child(value.clone())
        }))
}

/// Dim placeholder text.
pub(super) fn dim(text: &'static str) -> Div {
    div().text_color(color(theme::DIM)).child(text)
}

/// A dim footnote.
fn note(text: &'static str) -> Div {
    div()
        .mt(px(14.))
        .text_size(px(theme::FS_SMALL))
        .text_color(color(theme::DIM))
        .child(text)
}
