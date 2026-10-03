//! The replay strip: every call and model step as a tick on one rail, with turn markers and the
//! replay cursor; click to replay from there.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    Bounds, Context, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    ParentElement as _, Pixels, StatefulInteractiveElement as _, Styled as _, canvas, div, fill,
    point, px, relative, size,
};
use ichnos_model::labels::{clock, duration};

use super::Viewer;
use crate::theme::{self, MONO, color};

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
        let mut ticks: Vec<(f32, Hsla)> = Vec::new();
        if bounds.is_some() {
            for event in self.data().events() {
                let tick = match event.name() {
                    "PostToolUseFailure" => color(theme::RED),
                    "PreToolUse" if !event.is_main() => color(theme::PURPLE),
                    "PreToolUse" => color(theme::GREEN),
                    _ => continue,
                };
                ticks.push((fraction(event.epoch_ms()), tick));
            }
            for step in self.data().steps() {
                ticks.push((fraction(step.first_ms()), color(theme::MODEL_BAR)));
            }
        }
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
            (Some(b), None) => format!("{} · live", duration(b.t1 - b.t0)),
        };

        let strip_bounds: Rc<Cell<Bounds<Pixels>>> = Rc::default();
        let measured = Rc::clone(&strip_bounds);
        let button = |id: &'static str, text: &'static str| {
            div()
                .id(id)
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
                        button("prev", "◀")
                            .on_click(cx.listener(|this, _, _, cx| this.step(false, cx))),
                    )
                    .child(
                        button(
                            "play",
                            if self.state.playing() {
                                "❚❚"
                            } else {
                                "▶"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
                    )
                    .child(
                        button("live", "▶|")
                            .on_click(cx.listener(|this, _, _, cx| this.go_live(cx))),
                    ),
            )
            .child(
                div()
                    .id("strip")
                    .relative()
                    .flex_1()
                    .h(px(26.))
                    .cursor_crosshair()
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
                                for (at, tick) in &ticks {
                                    let x = bounds.origin.x + bounds.size.width * *at;
                                    let tick_bounds = Bounds::new(
                                        point(x, rail.origin.y + px(3.)),
                                        size(px(2.), px(8.)),
                                    );
                                    window.paint_quad(fill(tick_bounds, *tick));
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
                                    .text_size(px(theme::FS_TINY))
                                    .text_color(color(theme::MUTED))
                                    .child(n.to_string()),
                            )
                    })),
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
    }
}
