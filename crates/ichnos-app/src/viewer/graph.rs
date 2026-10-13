//! The graph: one card per agent, subagents under the agent that started them, joined by dashed
//! curves.
//!
//! Cards have fixed sizes, so the layout is computed here and the curves are painted from it.

use accesskit::Role;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, PathBuilder, PinchEvent,
    Pixels, Point, Rems, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _,
    Styled as _, canvas, div, point, px, relative, rems,
};
use ichnos_core::event::MAIN_AGENT;
use ichnos_model::call::Key;
use ichnos_model::labels::{duration, label, text, tokens};
use ichnos_model::model::{AgentBox, AgentGraph, NodeKind, SessionModel};

use gpui_kit::TestSupportExt as _;

use crate::zoom::zoom;

mod layout;

use self::layout::{TOOL_ROW, layout};

use super::{ToolSort, Viewer};
use crate::present::plural;
use crate::theme::{self, MONO, color};

/// How much one press of a zoom button or key zooms.
pub(super) const ZOOM_STEP: f32 = 1.2;
/// How much a pixel of ⌘-scroll zooms: 100 px is a factor of e.
const SCROLL_ZOOM: f32 = 0.01;

/// What the graph shows about one agent.
pub(super) struct Facts {
    pub(super) name: String,
    pub(super) main: bool,
    pub(super) dur: Option<i64>,
    pub(super) tool_ms: i64,
    pub(super) calls: usize,
    pub(super) prompt: String,
    pub(super) full_prompt: String,
    pub(super) returned: String,
    pub(super) spawned: String,
    pub(super) via: String,
    pub(super) status: String,
    pub(super) key: Option<Key>,
}

pub(super) fn facts(entry: &AgentBox, model: &SessionModel, viewer: &Viewer) -> Facts {
    let summaries = viewer.summaries();
    let node = entry.key().and_then(|key| model.find(key));
    let prompted: Vec<_> = model
        .turns()
        .iter()
        .copied()
        .filter(|&t| {
            matches!(
                model.node(t).kind(),
                NodeKind::Turn {
                    prompt: Some(_),
                    ..
                }
            )
        })
        .collect();
    let tool_ms = entry.tools().iter().map(|(_, t)| t.ms).sum();
    let calls = entry.tools().iter().map(|(_, t)| t.calls).sum();
    let end = model.end();
    let first = model.turns().first().and_then(|&t| model.node(t).start());
    if entry.id() == MAIN_AGENT {
        let returned = model
            .turns()
            .iter()
            .rev()
            .find_map(|&t| match model.node(t).kind() {
                NodeKind::Turn {
                    stop: Some(stop), ..
                } => Some(
                    stop.extra("last_assistant_message")
                        .map(text)
                        .unwrap_or_default(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        return Facts {
            name: "Main agent".to_owned(),
            main: true,
            dur: first.map(|first| end - first),
            tool_ms,
            calls,
            prompt: format!("Session root · {}", plural(prompted.len(), "user turn")),
            full_prompt: String::new(),
            returned,
            spawned: "session root".to_owned(),
            via: String::new(),
            status: if entry.running() { "running" } else { "idle" }.to_owned(),
            key: None,
        };
    }
    let node_ref = node.map(|id| model.node(id));
    let input = node_ref
        .and_then(|n| n.call())
        .map(|c| c.input().clone())
        .unwrap_or_default();
    let field = |name: &str| {
        input
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let turn = node.map(|id| model.turn_of(id));
    let turn_number = turn
        .and_then(|t| prompted.iter().position(|&p| p == t))
        .map(|i| i + 1);
    let returned = node_ref
        .and_then(|n| n.agent())
        .and_then(|a| a.stop_event())
        .and_then(|stop| stop.extra("last_assistant_message"))
        .map(text)
        .unwrap_or_default();
    let since = (entry.start().unwrap_or(0) - first.unwrap_or(0)).max(0);
    let spawned = match turn_number {
        Some(n) => format!("turn {n} · +{}", duration(since)),
        None => format!("+{}", duration(since)),
    };
    let via = match node_ref.and_then(|n| n.call()) {
        Some(call) => {
            let tool = call.tool_name().unwrap_or_default();
            match turn_number {
                Some(n) => format!("{tool} · turn {n}"),
                None => tool.to_owned(),
            }
        }
        None => "started by the agent itself".to_owned(),
    };
    let status = if entry.running() {
        "running".to_owned()
    } else if entry.errors() > 0 {
        format!("done · {}", plural(entry.errors(), "error"))
    } else {
        "done".to_owned()
    };
    Facts {
        name: entry.agent_type().unwrap_or("subagent").to_owned(),
        main: false,
        dur: entry
            .start()
            .map(|start| entry.end().unwrap_or(end) - start),
        tool_ms,
        calls,
        prompt: node_ref
            .and_then(|n| n.call())
            .and_then(|call| call.label())
            .map(str::to_owned)
            .or_else(|| field("prompt"))
            .or_else(|| node_ref.map(|n| label(n, summaries, model).text))
            .unwrap_or_default(),
        full_prompt: field("prompt").unwrap_or_default(),
        returned,
        spawned,
        via,
        status,
        key: entry.key().cloned(),
    }
}

impl Viewer {
    pub(super) fn agent_graph(&self) -> AgentGraph {
        let summaries = self.summaries();
        self.model
            .agent_graph(|node| label(node, summaries, &self.model).text)
    }

    pub(super) fn render_graph(&self, cx: &mut Context<Self>) -> AnyElement {
        let graph = self.agent_graph();
        let layout = layout(&graph);
        let position = |index: usize| layout.cards.iter().find(|card| card.0 == index).copied();
        let curves: Vec<(Point<f32>, Point<f32>)> = layout
            .edges
            .iter()
            .filter_map(|&(from, to)| {
                let (_, a, aw, ah) = position(from)?;
                let (_, b, bw, _) = position(to)?;
                Some((
                    Point {
                        x: a.x + aw / 2.,
                        y: a.y + ah,
                    },
                    Point {
                        x: b.x + bw / 2.,
                        y: b.y,
                    },
                ))
            })
            .collect();
        let labels: Vec<(Point<f32>, String)> = layout
            .edges
            .iter()
            .zip(&curves)
            .filter_map(|(&(_, to), &(a, b))| {
                let entry = graph.get(to)?;
                let via = facts(entry, &self.model, self).via;
                let mid = f32::midpoint(a.y, b.y);
                Some((
                    Point {
                        x: f32::midpoint(a.x, b.x) + (b.x - a.x) * 0.25,
                        y: mid + 8.,
                    },
                    via,
                ))
            })
            .collect();
        let sort = self.tool_sort;
        let pan = *self.state.graph_pan();
        let (pan_x, pan_y) = pan.offset();
        let scale = pan.zoom();

        // The graph is centered in the pane and moved from there by dragging or scrolling.
        div()
            .id("graph")
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .when_else(
                pan.is_dragging(),
                gpui_kit::Styled::cursor_grabbing,
                gpui_kit::Styled::cursor_grab,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.state.graph_pan_mut().start(pair(event.position));
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.dragging() && this.state.graph_pan_mut().drag_to(pair(event.position)) {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.state.graph_pan_mut().end();
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.state.graph_pan_mut().end();
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = pair(event.delta.pixel_delta(px(20.)));
                let pan = this.state.graph_pan_mut();
                // ⌘ or Ctrl turns scrolling into zooming, as in maps and design tools.
                if event.modifiers.platform || event.modifiers.control {
                    pan.zoom_by((delta.1 * SCROLL_ZOOM).exp());
                } else {
                    pan.scroll(delta);
                }
                cx.notify();
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| {
                this.state.graph_pan_mut().zoom_by(1. + event.delta);
                cx.notify();
            }))
            .child(zoom(
                scale,
                div()
                    .relative()
                    .flex_none()
                    .left(px(pan_x))
                    .top(px(pan_y))
                    .w(u(layout.width))
                    .h(u(layout.height))
                    .text_size(u(12.))
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, (), window, _| {
                                let at = |p: Point<f32>| -> Point<Pixels> {
                                    point(
                                        bounds.origin.x + px(p.x * scale),
                                        bounds.origin.y + px(p.y * scale),
                                    )
                                };
                                for &(a, b) in &curves {
                                    let mid = f32::midpoint(a.y, b.y);
                                    let mut path =
                                        PathBuilder::stroke(px(1.5)).dash_array(&[px(4.), px(3.)]);
                                    path.move_to(at(a));
                                    path.cubic_bezier_to(
                                        at(b),
                                        at(Point {
                                            x: a.x,
                                            y: mid + 20.,
                                        }),
                                        at(Point {
                                            x: b.x,
                                            y: mid - 20.,
                                        }),
                                    );
                                    if let Ok(path) = path.build() {
                                        window.paint_path(path, color(theme::PURPLE));
                                    }
                                }
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .children(labels.into_iter().map(|(at, via)| {
                        div()
                            .absolute()
                            .left(u(at.x - 60.))
                            .top(u(at.y - 9.))
                            .w(u(120.))
                            .flex()
                            .justify_center()
                            .child(
                                div()
                                    .px(u(7.))
                                    .py(u(2.))
                                    .bg(color(theme::PURPLE_BG))
                                    .border_1()
                                    .border_color(color(theme::PURPLE_LINE))
                                    .rounded(u(10.))
                                    .font_family(MONO)
                                    .text_size(u(10.))
                                    .text_color(color(theme::PURPLE_TEXT))
                                    .whitespace_nowrap()
                                    .child(via),
                            )
                    }))
                    .children(layout.cards.iter().filter_map(|&(index, at, w, h)| {
                        let entry = graph.get(index)?;
                        Some(self.render_card(entry, index, at, w, h, sort, cx))
                    })),
            ))
            .child(
                div()
                    .absolute()
                    .top(px(10.))
                    .right(px(10.))
                    .flex()
                    .flex_row()
                    .border_1()
                    .border_color(color(theme::LINE_2))
                    .rounded(px(theme::R_CONTROL))
                    .bg(color(theme::BG))
                    .text_size(px(theme::FS_SMALL))
                    .text_color(color(theme::BTN))
                    // Presses on the buttons don't start a drag.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .id("zoom-out")
                            .role(Role::Button)
                            .aria_label("Zoom out")
                            .px_2()
                            .py_1()
                            .cursor_pointer()
                            .hover(|s| s.text_color(color(theme::BRIGHT)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.graph_pan_mut().zoom_by(1. / ZOOM_STEP);
                                cx.notify();
                            }))
                            .child("−"),
                    )
                    .child(
                        div()
                            .id("zoom-reset")
                            .role(Role::Button)
                            .aria_label("Reset zoom")
                            .w(px(44.))
                            .py_1()
                            .flex()
                            .justify_center()
                            .font_family(MONO)
                            .cursor_pointer()
                            .hover(|s| s.text_color(color(theme::BRIGHT)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.graph_pan_mut().reset_zoom();
                                cx.notify();
                            }))
                            .child(format!("{:.0}%", scale * 100.)),
                    )
                    .child(
                        div()
                            .id("zoom-in")
                            .role(Role::Button)
                            .aria_label("Zoom in")
                            .px_2()
                            .py_1()
                            .border_r_1()
                            .border_color(color(theme::LINE_2))
                            .cursor_pointer()
                            .hover(|s| s.text_color(color(theme::BRIGHT)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.graph_pan_mut().zoom_by(ZOOM_STEP);
                                cx.notify();
                            }))
                            .child("+"),
                    )
                    .child(
                        div()
                            .id("center-graph")
                            .role(Role::Button)
                            .aria_label("Center graph")
                            .px_2()
                            .py_1()
                            .border_r_1()
                            .border_color(color(theme::LINE_2))
                            .cursor_pointer()
                            .hover(|s| s.text_color(color(theme::BRIGHT)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.graph_pan_mut().reset();
                                cx.notify();
                            }))
                            .child("Center"),
                    )
                    .child(
                        div()
                            .id("sort-time")
                            .role(Role::Button)
                            .aria_label("Sort tools by time")
                            .px_2()
                            .py_1()
                            .cursor_pointer()
                            .when(sort == ToolSort::Time, |d| {
                                d.text_color(color(theme::BRIGHT))
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tool_sort = ToolSort::Time;
                                cx.notify();
                            }))
                            .child("Sort tools: time"),
                    )
                    .child(
                        div()
                            .id("sort-count")
                            .role(Role::Button)
                            .aria_label("Sort tools by count")
                            .px_2()
                            .py_1()
                            .border_l_1()
                            .border_color(color(theme::LINE_2))
                            .cursor_pointer()
                            .when(sort == ToolSort::Count, |d| {
                                d.text_color(color(theme::BRIGHT))
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tool_sort = ToolSort::Count;
                                cx.notify();
                            }))
                            .child("count"),
                    ),
            )
            .into_any_element()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "a card's place comes from the layout"
    )]
    fn render_card(
        &self,
        entry: &AgentBox,
        index: usize,
        at: Point<f32>,
        width: f32,
        height: f32,
        sort: ToolSort,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let facts = facts(entry, &self.model, self);
        let selected = self.state.agent() == entry.id();
        let mut tools = entry.tools().to_vec();
        match sort {
            ToolSort::Time => tools.sort_by_key(|(_, t)| std::cmp::Reverse(t.ms)),
            ToolSort::Count => tools.sort_by_key(|(_, t)| std::cmp::Reverse(t.calls)),
        }
        let max_ms = tools.iter().map(|(_, t)| t.ms).max().unwrap_or(1).max(1);
        let accent = if facts.main {
            theme::BLUE
        } else {
            theme::PURPLE
        };
        let id = entry.id().to_owned();

        div()
            .id(("card", index))
            .role(Role::Button)
            .aria_label(facts.name.clone())
            .absolute()
            .left(u(at.x))
            .top(u(at.y))
            .w(u(width))
            .h(u(height))
            .flex()
            .flex_col()
            .bg(color(theme::CARD))
            .border_1()
            .rounded(u(theme::R_CARD))
            .overflow_hidden()
            .cursor_pointer()
            .border_color(color(if selected {
                accent
            } else if entry.running() {
                theme::BLUE_LINE
            } else {
                theme::LINE_2
            }))
            .hover(|s| s.border_color(color(theme::HOVER_LINE)))
            .on_click(cx.listener(move |this, _, _, cx| {
                // Releasing a drag that started on a card doesn't select it.
                if !this.state.graph_pan().was_drag() {
                    this.state.select_agent(&id);
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .pt(u(10.))
                    .child(agent_badge(facts.main))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color(theme::BRIGHT))
                            .child(facts.name.clone()),
                    )
                    .when(entry.errors() > 0, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .text_size(u(11.))
                                .text_color(color(theme::RED))
                                .child(format!("✕ {}", entry.errors())),
                        )
                    })
                    .when(entry.running(), |d| {
                        d.child(div().size(u(6.)).rounded(u(3.)).bg(color(theme::BLUE)))
                    })
                    .child(
                        div()
                            .ml_auto()
                            .flex_none()
                            .font_family(MONO)
                            .text_size(u(11.))
                            .text_color(color(theme::MUTED))
                            .child(facts.dur.map(duration).unwrap_or_default()),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .pt(u(2.))
                    .pb_2()
                    .truncate()
                    .text_size(u(11.))
                    .text_color(color(theme::MUTED))
                    .child(super::one_line(&facts.prompt)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .px_3()
                    .pb_2()
                    .when(tools.is_empty(), |d| {
                        d.child(
                            div()
                                .italic()
                                .text_color(color(theme::DIM))
                                .child("no tool calls yet"),
                        )
                    })
                    .children(tools.into_iter().map(|(name, tally)| {
                        #[expect(
                            clippy::cast_precision_loss,
                            reason = "millisecond totals are far below 2^52"
                        )]
                        let share = (tally.ms as f32 / max_ms as f32).max(0.01);
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .h(u(TOOL_ROW))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_1()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(name)
                                    .when(tally.errors > 0, |d| {
                                        d.child(
                                            div()
                                                .text_color(color(theme::RED))
                                                .child(format!("✕{}", tally.errors)),
                                        )
                                    }),
                            )
                            .child(numeric(28., theme::DIM).child(tally.calls.to_string()))
                            .child(
                                div()
                                    .w(u(80.))
                                    .flex_none()
                                    .h(u(5.))
                                    .rounded(u(2.))
                                    .bg(color(theme::SOFT_LINE))
                                    .child(
                                        div()
                                            .h(u(5.))
                                            .w(relative(share))
                                            .min_w(u(2.))
                                            .rounded(u(2.))
                                            .bg(color(if tally.errors > 0 {
                                                theme::RED_BAR
                                            } else {
                                                theme::GREEN_BAR
                                            })),
                                    ),
                            )
                            .child(numeric(56., theme::BTN).child(duration(tally.ms)))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .px_3()
                    .py(u(7.))
                    .border_t_1()
                    .border_color(color(theme::SOFT_LINE))
                    .font_family(MONO)
                    .text_size(u(10.5))
                    .text_color(color(theme::MUTED))
                    .whitespace_nowrap()
                    .child(format!("model {}", duration(entry.model_ms())))
                    .child(format!("tools {}", duration(facts.tool_ms)))
                    .child(format!("{} out", tokens(entry.output_tokens())))
                    .child(
                        div()
                            .ml_auto()
                            .text_color(color(theme::FG))
                            .child(format!("{} calls", facts.calls)),
                    ),
            )
            .test_support()
            .into_any_element()
    }
}

/// The `main` or `sub` badge of a card.
pub(super) fn agent_badge(main: bool) -> gpui_kit::Div {
    let (text, fg, line) = if main {
        ("main", theme::BLUE_TEXT, theme::BLUE_LINE)
    } else {
        ("sub", theme::PURPLE_TEXT, theme::PURPLE_LINE)
    };
    div()
        .flex_none()
        .font_family(MONO)
        .text_size(u(10.))
        .border_1()
        .border_color(color(line))
        .rounded(u(theme::R_TAG))
        .px(u(5.))
        .text_color(color(fg))
        .child(SharedString::from(text))
}

/// A point in pixels as plain numbers, for [`crate::view_state::Pan`].
fn pair(point: Point<Pixels>) -> (f32, f32) {
    (f32::from(point.x), f32::from(point.y))
}

/// A graph length: layout units are pixels at 100%, written as rems so [`zoom`] scales them.
fn u(pixels: f32) -> Rems {
    rems(pixels / BASE_REM)
}

/// The rem size layout units are written against: GPUI's default.
const BASE_REM: f32 = 16.;

/// A right-aligned mono number column that scales with the graph.
fn numeric(width: f32, text: crate::theme::Tok) -> gpui_kit::Div {
    div()
        .w(u(width))
        .flex_none()
        .flex()
        .justify_end()
        .font_family(MONO)
        .text_size(u(11.))
        .whitespace_nowrap()
        .text_color(color(text))
}
