//! "Ask agent": write a question, then copy a prompt that tells an agent where this session's
//! database is and how to query it. The agent answers; Ichnos only builds the prompt.

use accesskit::Role;
use std::path::PathBuf;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{self, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use ichnos_model::ask::{AskArgs, STARTERS, ask_prompt, focus_of};
use ichnos_model::labels::label;

use super::toolbar::cwd_of;
use super::{Viewer, one_line};
use crate::theme::{self, MONO, color};

/// Asks the viewer to close the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Close;

/// The question box of the Ask panel; the viewer draws the rest.
pub struct AskPanel {
    question: Entity<TextareaState>,
}

impl EventEmitter<Close> for AskPanel {}

impl AskPanel {
    /// A panel with an empty, focused question box.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let question = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What do you want to know? For example: why did the commit fail?")
        });
        question.update(cx, |question, cx| question.focus(window, cx));
        Self { question }
    }

    /// The question box.
    pub(super) fn question(&self) -> &Entity<TextareaState> {
        &self.question
    }
}

impl Render for AskPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Textarea::new(&self.question).h(px(64.))
    }
}

impl Viewer {
    pub(super) fn render_ask(&self, ask: &Entity<AskPanel>, cx: &mut Context<Self>) -> AnyElement {
        let question = ask.read(cx).question().read(cx).value().to_string();
        let session = self
            .session
            .as_ref()
            .map(|s| s.as_str().to_owned())
            .unwrap_or_default();
        let path: Option<PathBuf> = self
            .session
            .as_ref()
            .and_then(|id| self.sessions.iter().find(|s| s.id() == id))
            .map(|s| s.path().to_owned());
        let cwd = cwd_of(self);
        let selected = self.state.selected().and_then(|key| self.model.find(key));
        let focus = selected
            .filter(|_| self.ask_focus)
            .map(|id| focus_of(id, &self.model, self.summaries()));
        let prompt = ask_prompt(&AskArgs {
            session: &session,
            path: path.as_deref(),
            cwd: (!cwd.is_empty()).then_some(cwd.as_str()),
            focus: focus.as_deref(),
            question: &question,
        });
        let copied = self.is_copied("ask-prompt");
        let focus_label = selected
            .map(|id| one_line(&label(self.model.node(id), self.summaries(), &self.model).text));
        let ask_focus = self.ask_focus;
        let close = ask.clone();
        let starters_target = ask.clone();
        let copy_prompt = prompt.clone();

        div()
            .id("ask-panel")
            .absolute()
            .top(px(48.))
            .right(px(12.))
            .w(px(560.))
            .max_h(px(640.))
            .flex()
            .flex_col()
            .bg(color(theme::CARD))
            .border_1()
            .border_color(color(theme::LINE_2))
            .rounded(px(theme::R_CARD))
            .shadow_lg()
            .on_action(cx.listener(move |_, _: &input::Escape, _, cx| {
                close.update(cx, |_, cx| cx.emit(Close));
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px(px(14.))
                    .py_3()
                    .border_b_1()
                    .border_color(color(theme::LINE))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(theme::FS_BASE))
                            .text_color(color(theme::BRIGHT))
                            .child("Ask an agent about this session"),
                    )
                    .child(
                        div()
                            .id("ask-close")
                            .role(Role::Button)
                            .aria_label("Close ask panel")
                            .ml_auto()
                            .text_size(px(theme::FS_TITLE))
                            .text_color(color(theme::DIM))
                            .cursor_pointer()
                            .hover(|s| s.text_color(color(theme::FG)))
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_ask(false, window, cx)))
                            .child("×"),
                    ),
            )
            .child(
                div()
                    .id("ask-body")
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .px(px(14.))
                    .py_3()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(ask.clone())
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .gap_1p5()
                            .children(STARTERS.iter().enumerate().map(|(i, starter)| {
                                let target = starters_target.clone();
                                div()
                                    .id(("starter", i))
                                    .role(Role::Button)
                                    .aria_label(*starter)
                                    .px_2()
                                    .py(px(3.))
                                    .border_1()
                                    .border_color(color(theme::LINE_2))
                                    .rounded(px(12.))
                                    .text_size(px(theme::FS_SMALL))
                                    .text_color(color(theme::MUTED))
                                    .cursor_pointer()
                                    .hover(|s| s.text_color(color(theme::FG)).border_color(color(theme::HOVER_LINE)))
                                    .on_click(cx.listener(move |_, _, window, cx| {
                                        let question = target.read(cx).question().clone();
                                        question.update(cx, |question, cx| {
                                            question.set_value((*starter).to_owned(), window, cx);
                                        });
                                    }))
                                    .child(*starter)
                            })),
                    )
                    .when_some(focus_label, |d, focus_label| {
                        d.child(
                            Checkbox::new("ask-focus")
                                .checked(ask_focus)
                                .label(format!("About the selected row: {focus_label}"))
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.ask_focus = *checked;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(super::inspector::heading("Prompt").mt_0())
                    .child(
                        div()
                            .id("ask-prompt")
                            .max_h(px(220.))
                            .overflow_y_scroll()
                            .px(px(10.))
                            .py_2()
                            .rounded(px(theme::R_CONTROL))
                            .border_1()
                            .border_color(color(theme::LINE))
                            .bg(color(theme::FIELD))
                            .font_family(MONO)
                            .text_size(px(theme::FS_SMALL))
                            .text_color(color(theme::TEXT_2))
                            .child(prompt),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .px(px(14.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(color(theme::LINE))
                    .child(
                        div()
                            .text_size(px(theme::FS_SMALL))
                            .text_color(color(theme::DIM))
                            .child("Paste it into your coding agent. It reads the database with sqlite3."),
                    )
                    .child(
                        div()
                            .id("ask-copy")
                            .role(Role::Button)
                            .aria_label("Copy prompt")
                            .ml_auto()
                            .flex_none()
                            .px(px(14.))
                            .py_1p5()
                            .rounded(px(theme::R_CONTROL))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(color(theme::WHITE))
                            .bg(color(if copied { theme::GREEN_BAR } else { theme::BLUE }))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.copy(SharedString::from("ask-prompt"), copy_prompt.clone(), cx);
                            }))
                            .child(if copied { "Copied ✓" } else { "Copy prompt" }),
                    ),
            )
            .into_any_element()
    }
}
