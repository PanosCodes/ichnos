//! The toolbar: session picker, view switch, model steps, filter, error jump, expand/collapse,
//! Ask, the panel toggle and help; the stats line over the main pane; and the keyboard shortcuts
//! overlay.

use accesskit::{Role, Toggled};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{self, Input};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use ichnos_core::catalog::project_of;
use ichnos_model::labels::{duration, tokens};

use super::{Viewer, short_id};
use crate::present::plural;
use crate::theme::{self, MONO, color, color_alpha};
use crate::view_state::{View, failed_calls};

/// Window width, in pixels, below which the toolbar drops what it can do without.
pub(super) const NARROW: f32 = 1100.;

impl Viewer {
    pub(super) fn render_toolbar(
        &mut self,
        window: &mut Window,
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
            .and_then(|ms| ichnos_core::local_time(ms, "%H:%M"))
            .unwrap_or_default();
        let id = self.session.as_ref().map(|id| short_id(id.as_str()));
        let picker_label = if project.is_empty() {
            id.map_or("no session", |id| id).to_owned()
        } else {
            project.clone()
        };
        let picker_extra = [
            id.filter(|_| !project.is_empty()).unwrap_or_default(),
            when.as_str(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .fold(String::new(), |extra, part| extra + " · " + part);

        let narrow = f32::from(window.viewport_size().width) < NARROW;
        let errors = failed_calls(&self.model).len();
        let tree = self.state.view() == View::Tree;
        let panel_open = self.state.panel().is_open();
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
                    .id("picker")
                    .role(Role::Button)
                    .aria_label("Switch session")
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
                    .active(|s| s.opacity(0.7))
                    .hover(|s| {
                        s.text_color(color(theme::BRIGHT))
                            .border_color(color(theme::HOVER_LINE))
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_switcher(window, cx)))
                    .child(picker_label)
                    .when(!narrow, |d| {
                        d.child(div().text_color(color(theme::DIM)).child(picker_extra))
                            .child(kbd("⌘K"))
                    }),
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
                            .role(Role::Tab)
                            .aria_label(view.label())
                            .aria_selected(on)
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
                        div().flex_1().min_w_0().child(
                            Input::new(&self.filter_input)
                                .appearance(false)
                                .cleanable(false),
                        ),
                    )
                    .child(if has_query {
                        div()
                            .id("clear-filter")
                            .role(Role::Button)
                            .aria_label("Clear filter")
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
            .when(errors > 0, |d| {
                d.child(
                    div()
                        .id("errors")
                        .role(Role::Button)
                        .aria_label(format!("{}: jump to the next", plural(errors, "error")))
                        .flex()
                        .flex_none()
                        .items_center()
                        .h(px(theme::CONTROL_HEIGHT))
                        .px(px(10.))
                        .border_1()
                        .rounded(px(theme::R_CONTROL))
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .border_color(color(theme::RED_LINE))
                        .text_color(color(theme::RED_TEXT))
                        .hover(|s| s.bg(color(theme::RED_BG)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.on_next_error(&super::NextError, window, cx);
                        }))
                        .child(if narrow {
                            format!("{errors} ↓")
                        } else {
                            format!("{} · next ↓", plural(errors, "error"))
                        }),
                )
            })
            .when(tree, |d| {
                let viewer = cx.entity().downgrade();
                d.child(
                    Button::new("tree-options")
                        .outline()
                        .compact()
                        .small()
                        .text_size(px(theme::FS_SMALL))
                        .label("Options")
                        .accessibility_label("Tree options")
                        .dropdown_caret(true)
                        .dropdown_menu(move |menu, _, _| {
                            let (steps, open, shut) =
                                (viewer.clone(), viewer.clone(), viewer.clone());
                            menu.item(
                                PopupMenuItem::new("Model steps")
                                    .checked(show_model)
                                    .on_click(move |_, _, cx| {
                                        steps
                                            .update(cx, |this, cx| {
                                                this.state.set_show_model(!show_model);
                                                this.refresh();
                                                cx.notify();
                                            })
                                            .ok();
                                    }),
                            )
                            .item(PopupMenuItem::separator())
                            .item(PopupMenuItem::new("Expand all").on_click(move |_, _, cx| {
                                open.update(cx, |this, cx| this.set_all_open(true, cx)).ok();
                            }))
                            .item(
                                PopupMenuItem::new("Collapse all").on_click(move |_, _, cx| {
                                    shut.update(cx, |this, cx| this.set_all_open(false, cx))
                                        .ok();
                                }),
                            )
                        }),
                )
            })
            .child(
                div()
                    .id("ask")
                    .role(Role::Button)
                    .aria_label("Ask about this session")
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
                icon_button(
                    "panel-toggle",
                    if panel_open {
                        "Hide inspector"
                    } else {
                        "Show inspector"
                    },
                    panel_icon(panel_open),
                )
                .aria_keyshortcuts("Meta+\\")
                .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(cx))),
            )
            .child(
                icon_button("help", "Keyboard shortcuts", "?")
                    .aria_keyshortcuts("?")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_help(cx))),
            )
    }

    pub(super) fn render_stats(&self, narrow: bool) -> impl IntoElement {
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
        .when(!narrow, |d| {
            d.child(div().ml_auto().text_color(color(theme::FG)).child(format!(
                "model {} · tools {}",
                duration(stats.model_ms),
                duration(tool_ms)
            )))
        })
    }

    /// The keyboard shortcuts overlay; a click outside the card or Esc closes it.
    pub(super) fn render_help(cx: &mut Context<Self>) -> impl IntoElement {
        let group = |title: &'static str, keys: &[(&'static str, &'static str)]| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(theme::FS_SMALL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color(theme::MUTED))
                        .child(title),
                )
                .children(keys.iter().map(|&(key, what)| {
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .w(px(72.))
                                .flex_none()
                                .font_family(MONO)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(color(theme::BTN))
                                .child(key),
                        )
                        .child(what)
                }))
        };
        div()
            .id("help-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(color_alpha(0x0000_0080))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.help = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("help")
                    .role(Role::Dialog)
                    .aria_label("Keyboard shortcuts")
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w(px(520.))
                    .p_5()
                    .bg(color(theme::CARD))
                    .border_1()
                    .border_color(color(theme::LINE_2))
                    .rounded(px(theme::R_POPUP))
                    .shadow_lg()
                    .text_color(color(theme::FG))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(theme::FS_TITLE))
                                    .text_color(color(theme::BRIGHT))
                                    .child("Keyboard shortcuts"),
                            )
                            .child(
                                icon_button("help-close", "Close keyboard shortcuts", "×")
                                    .ml_auto()
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_help(cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_6()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .gap_4()
                                    .child(group(
                                        "Rows",
                                        &[
                                            ("j k ↓ ↑", "Move"),
                                            ("↵", "Expand or collapse"),
                                            ("e", "Next error"),
                                            ("/", "Filter"),
                                        ],
                                    ))
                                    .child(group(
                                        "Replay",
                                        &[("space", "Play or pause"), ("← →", "Step")],
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .gap_4()
                                    .child(group(
                                        "Views",
                                        &[
                                            ("t g b", "Tree, graph, table"),
                                            ("⌘K", "Switch session"),
                                            ("⌘\\", "Show or hide inspector"),
                                        ],
                                    ))
                                    .child(group(
                                        "Graph",
                                        &[("⌘= ⌘-", "Zoom in, out"), ("⌘0", "Actual size")],
                                    ))
                                    .child(group("", &[("?", "This list"), ("esc", "Close")])),
                            ),
                    ),
            )
    }
}

/// A toolbar toggle whose text stays put and whose state shows by fill and to assistive tech.
pub(super) fn toggle(
    id: &'static str,
    label: &'static str,
    text: &'static str,
    on: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .h(px(theme::CONTROL_HEIGHT))
        .px(px(9.))
        .border_1()
        .rounded(px(theme::R_CONTROL))
        .whitespace_nowrap()
        .text_size(px(theme::FS_SMALL))
        .cursor_pointer()
        .active(|s| s.opacity(0.7))
        .when_else(
            on,
            |d| {
                d.bg(color(theme::SEL))
                    .border_color(color(theme::BLUE_LINE))
                    .text_color(color(theme::BLUE_TEXT))
            },
            |d| {
                d.border_color(color(theme::LINE_2))
                    .text_color(color(theme::MUTED))
                    .hover(|s| s.text_color(color(theme::FG)))
            },
        )
        .child(if on { "✓" } else { "○" })
        .child(text)
}

/// A square toolbar button showing `glyph`, named `label` for assistive tech.
pub(super) fn icon_button(
    id: &'static str,
    label: &'static str,
    glyph: impl IntoElement,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .group(id)
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(theme::CONTROL_HEIGHT))
        .border_1()
        .border_color(color(theme::LINE_2))
        .rounded(px(theme::R_CONTROL))
        .text_color(color(theme::MUTED))
        .cursor_pointer()
        .active(|s| s.opacity(0.7))
        .hover(|s| {
            s.text_color(color(theme::BRIGHT))
                .border_color(color(theme::HOVER_LINE))
        })
        .child(glyph)
}

/// A window with its right-hand panel filled, drawn rather than set as `◨`: that glyph comes
/// from a fallback font and sits off-centre in the button. It lights up with its button.
fn panel_icon(open: bool) -> gpui_kit::Div {
    let ink = if open { theme::BRIGHT } else { theme::MUTED };
    div()
        .flex()
        .flex_row()
        .justify_end()
        .w(px(14.))
        .h(px(12.))
        .border_1()
        .rounded(px(2.))
        .border_color(color(ink))
        .group_hover("panel-toggle", |s| s.border_color(color(theme::BRIGHT)))
        .child(
            div()
                .w(px(5.))
                .h_full()
                .bg(color(ink))
                .group_hover("panel-toggle", |s| s.bg(color(theme::BRIGHT))),
        )
}

/// A small key hint box.
pub(super) fn kbd(text: &'static str) -> gpui_kit::Div {
    div()
        .flex_none()
        .font_family(MONO)
        .text_size(px(theme::FS_SMALL))
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
