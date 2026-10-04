//! The replay strip: every call and model step as a tick on one rail, with turn markers and the
//! replay cursor; click to replay from there, hover a tick to see what it was. Its right end holds
//! the live status: the connection pill and the follow and auto-scroll toggles.

use accesskit::Role;
use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Bounds, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, StatefulInteractiveElement as _,
    Styled as _, canvas, deferred, div, fill, point, px, relative, size,
};
use ichnos_model::call::{Key, key_of};
use ichnos_model::labels::{clock, duration, label};

use super::toolbar::toggle;
use super::{Viewer, one_line};
use crate::live::Connection;
use crate::theme::{self, MONO, color};
use crate::view_state::{View, nearest_tick};

/// How far, in pixels, the pointer may be from a tick and still hover it.
const HOVER_REACH: f32 = 6.;

/// What a tick on the strip stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TickKind {
    /// A main-agent tool call starting.
    Call,
    /// A subagent tool call starting.
    Subagent,
    /// A tool call failing.
    Failure,
    /// A model step.
    Model,
}

/// One tick on the strip.
struct Tick {
    /// Where on the strip, as a fraction of its width.
    at: f32,
    ms: i64,
    kind: TickKind,
    /// The entry it belongs to; model steps have none.
    key: Option<Key>,
}

impl TickKind {
    fn color(self) -> Hsla {
        color(match self {
            Self::Call => theme::GREEN,
            Self::Subagent => theme::PURPLE,
            Self::Failure => theme::RED,
            Self::Model => theme::MODEL_BAR,
        })
    }
}

impl Viewer {
    pub(super) fn render_replay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds;
        let replay_at = self.state.replay_at();
        let span = bounds.map_or(1, |b| (b.t1 - b.t0).max(1));
        let t0 = bounds.map_or(0, |b| b.t0);
        #[expect(
            clippy::cast_precision_loss,
            reason = "millisecond offsets are far below 2^52"
        )]
        let fraction = move |ms: i64| ((ms - t0) as f32 / span as f32).clamp(0., 1.);

        // Ticks: tool calls by agent, failures in red, model steps in grey.
        let mut ticks: Vec<Tick> = Vec::new();
        if bounds.is_some() {
            for event in self.data().events() {
                let kind = match event.name() {
                    "PostToolUseFailure" => TickKind::Failure,
                    "PreToolUse" if !event.is_main() => TickKind::Subagent,
                    "PreToolUse" => TickKind::Call,
                    _ => continue,
                };
                ticks.push(Tick {
                    at: fraction(event.epoch_ms()),
                    ms: event.epoch_ms(),
                    kind,
                    key: Some(key_of(event)),
                });
            }
            for step in self.data().steps() {
                ticks.push(Tick {
                    at: fraction(step.first_ms()),
                    ms: step.first_ms(),
                    kind: TickKind::Model,
                    key: None,
                });
            }
        }
        let hovered = self.replay_hover.and_then(|(at, reach)| {
            let places: Vec<f32> = ticks.iter().map(|tick| tick.at).collect();
            let tick = &ticks[nearest_tick(&places, at, reach)?];
            Some((
                tick.at,
                format!("{} · {}", clock(tick.ms), self.tick_label(tick)),
            ))
        });
        let painted: Vec<(f32, TickKind)> = ticks.iter().map(|tick| (tick.at, tick.kind)).collect();
        let marks: Vec<(f32, usize)> = if bounds.is_some() {
            self.model
                .turns()
                .iter()
                .map(|&t| self.model.node(t))
                .filter(|t| {
                    matches!(
                        t.kind(),
                        ichnos_model::model::NodeKind::Turn {
                            prompt: Some(_),
                            ..
                        }
                    )
                })
                .enumerate()
                .map(|(i, t)| (fraction(t.start().unwrap_or(t0)), i + 1))
                .collect()
        } else {
            Vec::new()
        };
        let label = match (bounds, replay_at) {
            (None, _) => "—".to_owned(),
            (Some(b), Some(at)) => format!("{} · +{}", clock(at), duration(at - b.t0)),
            (Some(b), None) => duration(b.t1 - b.t0),
        };

        let strip_bounds: Rc<Cell<Bounds<Pixels>>> = Rc::default();
        let measured = Rc::clone(&strip_bounds);
        let button = |id: &'static str, label: &'static str, text: &'static str| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .px(px(7.))
                .py(px(2.))
                .border_1()
                .border_color(color(theme::LINE_2))
                .rounded(px(theme::R_CONTROL))
                .cursor_pointer()
                .hover(|s| {
                    s.text_color(color(theme::BRIGHT))
                        .border_color(color(theme::HOVER_LINE))
                })
                .child(text)
        };

        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(40.))
            .px_3()
            .border_b_1()
            .border_color(color(theme::LINE))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(2.))
                    .text_color(color(theme::BTN))
                    .text_size(px(theme::FS_SMALL))
                    .child(
                        button("prev", "Step back", "◀")
                            .on_click(cx.listener(|this, _, _, cx| this.step(false, cx))),
                    )
                    .child(
                        button(
                            "play",
                            if self.state.playing() {
                                "Pause"
                            } else {
                                "Play"
                            },
                            if self.state.playing() {
                                "❚❚"
                            } else {
                                "▶"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
                    )
                    .child(
                        button("live", "Go live", "▶|")
                            .on_click(cx.listener(|this, _, _, cx| this.go_live(cx))),
                    ),
            )
            .child(
                div()
                    .id("strip")
                    .role(Role::Slider)
                    .aria_label("Replay position")
                    .aria_value(label.clone())
                    .aria_keyshortcuts("ArrowLeft ArrowRight Space")
                    .relative()
                    .flex_1()
                    .h(px(26.))
                    .cursor_crosshair()
                    .on_mouse_move(cx.listener({
                        let rail = Rc::clone(&strip_bounds);
                        move |this, event: &MouseMoveEvent, _, cx| {
                            let rail = rail.get();
                            let width = f32::from(rail.size.width).max(1.);
                            let at = f32::from(event.position.x - rail.origin.x) / width;
                            this.replay_hover = Some((at, HOVER_REACH / width));
                            cx.notify();
                        }
                    }))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !hovered {
                            this.replay_hover = None;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            let Some(b) = this.bounds else {
                                return;
                            };
                            let rail = strip_bounds.get();
                            let width = f32::from(rail.size.width).max(1.);
                            let x =
                                (f32::from(event.position.x - rail.origin.x) / width).clamp(0., 1.);
                            #[expect(
                                clippy::cast_possible_truncation,
                                clippy::cast_precision_loss,
                                reason = "a pixel fraction of a millisecond span"
                            )]
                            let ms = b.t0 + (x * (b.t1 - b.t0) as f32).round() as i64;
                            this.seek(ms, cx);
                        }),
                    )
                    .child(
                        canvas(
                            move |bounds, _, _| measured.set(bounds),
                            move |bounds, (), window, _| {
                                let rail = Bounds::new(
                                    point(bounds.origin.x, bounds.origin.y + px(10.)),
                                    size(bounds.size.width, px(14.)),
                                );
                                window.paint_quad(
                                    fill(rail, color(theme::FIELD)).corner_radii(px(3.)),
                                );
                                for &(at, kind) in &painted {
                                    let x = bounds.origin.x + bounds.size.width * at;
                                    // Failures run the rail's full height, so they stand out
                                    // by shape as well as by color.
                                    let (top, height) = if kind == TickKind::Failure {
                                        (px(0.), px(14.))
                                    } else {
                                        (px(3.), px(8.))
                                    };
                                    let tick_bounds = Bounds::new(
                                        point(x, rail.origin.y + top),
                                        size(px(2.), height),
                                    );
                                    window.paint_quad(fill(tick_bounds, kind.color()));
                                }
                                let cursor_x = match replay_at {
                                    Some(at) => bounds.origin.x + bounds.size.width * fraction(at),
                                    None => bounds.origin.x + bounds.size.width - px(2.),
                                };
                                let cursor = Bounds::new(
                                    point(cursor_x, bounds.origin.y + px(6.)),
                                    size(px(2.), bounds.size.height - px(6.)),
                                );
                                window.paint_quad(fill(cursor, color(theme::BLUE)));
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .children(marks.into_iter().map(|(at, n)| {
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(at))
                            .border_l_1()
                            .border_color(color(theme::MARK))
                            .child(
                                div()
                                    .absolute()
                                    .left(px(3.))
                                    .top(px(-1.))
                                    .font_family(MONO)
                                    .text_size(px(theme::FS_SMALL))
                                    .text_color(color(theme::MUTED))
                                    .child(n.to_string()),
                            )
                    }))
                    .when_some(hovered, |d, (at, text)| d.child(tooltip(at, text))),
            )
            .child(
                div()
                    .min_w(px(90.))
                    .font_family(MONO)
                    .text_size(px(theme::FS_SMALL))
                    .whitespace_nowrap()
                    .text_color(color(if replay_at.is_some() {
                        theme::BLUE_TEXT
                    } else {
                        theme::MUTED
                    }))
                    .child(label),
            )
            .child(self.render_live(cx))
    }

    /// The connection pill and the follow and auto-scroll toggles.
    fn render_live(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        let follow = self.follow_newest;
        let tail = self.state.tail();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_1p5()
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
            .child(
                toggle("follow", "Follow newest", follow)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if follow {
                            this.follow_newest = false;
                            cx.notify();
                        } else {
                            this.open_session(None, None, cx);
                        }
                    }))
                    .test_support(),
            )
            .when(self.state.view() != View::Graph, |d| {
                d.child(toggle("tail", "Auto-scroll", tail).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.state.set_tail(!tail);
                        if !tail {
                            this.tree_scroll.scroll_to_bottom();
                            this.table_scroll.scroll_to_bottom();
                        }
                        cx.notify();
                    },
                )))
            })
    }

    /// What `tick` stands for, in one line.
    fn tick_label(&self, tick: &Tick) -> String {
        let Some(node) = tick.key.as_ref().and_then(|key| self.model.find(key)) else {
            return "Model step".to_owned();
        };
        let text = one_line(&label(self.model.node(node), self.summaries(), &self.model).text);
        if tick.kind == TickKind::Failure {
            format!("Failed: {text}")
        } else {
            text
        }
    }
}

/// The hovered tick's label under the strip, kept on screen at either end and drawn above the
/// rows.
fn tooltip(at: f32, text: String) -> impl IntoElement {
    deferred(
        div()
            .absolute()
            .top(px(28.))
            .when_else(
                at > 0.6,
                |d| d.right(relative(1. - at)),
                |d| d.left(relative(at)),
            )
            .max_w(px(420.))
            .px_2()
            .py_1()
            .bg(color(theme::CARD))
            .border_1()
            .border_color(color(theme::LINE_2))
            .rounded(px(theme::R_CONTROL))
            .shadow_lg()
            .font_family(MONO)
            .text_size(px(theme::FS_SMALL))
            .text_color(color(theme::FG))
            .truncate()
            .whitespace_nowrap()
            .child(text),
    )
}
