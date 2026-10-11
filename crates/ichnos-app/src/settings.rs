//! The settings window: a small fixed-size window laid out like a macOS preferences pane, with a
//! row of icon tabs on top and a form of right-aligned section names beside their controls.

use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Role, Toggled};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Action, AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable,
    FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Pixels,
    Render, StatefulInteractiveElement as _, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowOptions, actions, canvas, div, linear_color_stop, linear_gradient, point, px, size,
};

use crate::summarizers::{Availability, Choice};
use crate::theme::{self, color};
use crate::viewer::Viewer;

/// Width of the column of section names, which are right-aligned in it.
const SECTION_WIDTH: f32 = 84.;
/// Where a control's small print starts: under its label, past the checkbox.
const ABOUT_INDENT: f32 = 24.;
/// Side of the checkbox.
const BOX: f32 = 16.;
/// Size of the summarizer pop-up button, like a regular-size `NSPopUpButton`.
const POPUP_WIDTH: f32 = 220.;
const POPUP_HEIGHT: f32 = 24.;
/// Space between the pop-up button and its menu.
const POPUP_GAP: f32 = 2.;

/// Key context of the settings window.
const CONTEXT: &str = "IchnosSettings";

actions!(
    ichnos_settings,
    [
        /// Close the settings window.
        CloseSettings,
        /// Show the General tab.
        ShowGeneral,
        /// Show the About tab.
        ShowAbout,
    ]
);

/// Register the settings window's keyboard shortcuts.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-w", CloseSettings, Some(CONTEXT)),
        KeyBinding::new("escape", CloseSettings, Some(CONTEXT)),
        KeyBinding::new("cmd-1", ShowGeneral, Some(CONTEXT)),
        KeyBinding::new("cmd-2", ShowAbout, Some(CONTEXT)),
    ]);
}

/// Open the settings window for `viewer`, or `None` when the window can't be opened.
pub fn open(viewer: &Entity<Viewer>, cx: &mut App) -> Option<AnyWindowHandle> {
    let bounds = Bounds::centered(None, size(px(520.), px(280.)), cx);
    let viewer = viewer.clone();
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Settings".into()),
                ..TitlebarOptions::default()
            }),
            is_resizable: false,
            is_minimizable: false,
            ..WindowOptions::default()
        },
        cx,
        |window, cx| cx.new(|cx| Settings::new(viewer, window, cx)),
    )
    .inspect_err(|err| eprintln!("could not open the settings window: {err}"))
    .ok()?;
    Some(handle)
}

/// The tabs of the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    About,
}

impl Tab {
    const ALL: [Self; 2] = [Self::General, Self::About];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::About => "About",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::General => IconName::Settings,
            Self::About => IconName::Info,
        }
    }
}

/// Pick the summarizer at this index of the pop-up menu.
#[derive(Debug, Clone, PartialEq, Eq, Action)]
#[action(namespace = ichnos_settings, no_json)]
pub(crate) struct PickSummarizer {
    pub(crate) index: usize,
}

/// The settings window's root view.
pub struct Settings {
    focus: FocusHandle,
    viewer: Entity<Viewer>,
    tab: Tab,
    /// The summarizers to offer, each with whether it can run here (`None` until checked).
    summarizers: Vec<(Choice, Option<Availability>)>,
    /// Where the summarizer pop-up button was last drawn, to open its menu there.
    popup_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl Settings {
    pub(crate) fn new(viewer: Entity<Viewer>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The controls follow the viewer, which can also change on its own.
        cx.observe(&viewer, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let (choices, probe) = viewer.read(cx).summarizers();
        let choices = choices.to_vec();
        let summarizers = choices
            .iter()
            .map(|choice| (choice.clone(), None))
            .collect();
        // Checking can start processes, so it runs in the background and fills in later.
        let check = cx.background_spawn(async move {
            choices
                .into_iter()
                .map(|choice| {
                    let availability = probe(&choice);
                    (choice, Some(availability))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let checked = check.await;
            this.update(cx, |this, cx| {
                this.summarizers = checked;
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            focus,
            viewer,
            tab: Tab::General,
            summarizers,
            popup_bounds: Rc::default(),
        }
    }

    /// Open the summarizer menu as a native macOS menu under the pop-up button: the current one
    /// checked, the ones this Mac can't run greyed out.
    fn open_summarizer_menu(&self, window: &mut Window, cx: &mut App) {
        let current = self.viewer.read(cx).summarizer();
        let menu = self.summarizers.iter().enumerate().fold(
            NativeMenu::new(),
            |menu, (index, (choice, availability))| {
                let action = Box::new(PickSummarizer { index });
                if Some(choice) == current {
                    menu.menu_with_check(choice.label(), true, action)
                } else {
                    let missing = matches!(availability, Some(Availability::Missing(_)));
                    menu.menu_with_disabled(choice.label(), missing, action)
                }
            },
        );
        let Some(bounds) = self.popup_bounds.get() else {
            return;
        };
        menu.show(
            point(bounds.origin.x, bounds.bottom() + px(POPUP_GAP)),
            window,
            cx,
        );
    }

    fn pick_summarizer(&mut self, pick: &PickSummarizer, cx: &mut Context<Self>) {
        let Some((choice, _)) = self.summarizers.get(pick.index) else {
            return;
        };
        let choice = choice.clone();
        self.viewer
            .update(cx, |viewer, cx| viewer.set_summarizer(choice, cx));
    }

    /// The summarizer row's control: a macOS pop-up button, and why the chosen summarizer can't
    /// run when it can't.
    fn render_summarizer(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(current) = self.viewer.read(cx).summarizer().cloned() else {
            return div()
                .text_color(color(theme::MUTED))
                .child("Off: Ichnos was started with --no-summaries.")
                .into_any_element();
        };
        let missing =
            self.summarizers
                .iter()
                .find_map(|(choice, availability)| match availability {
                    Some(Availability::Missing(why)) if *choice == current => Some(*why),
                    _ => None,
                });
        let bounds = self.popup_bounds.clone();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id("summarizer")
                    .role(Role::ComboBox)
                    .aria_label(format!("Summarizer: {}", current.label()))
                    .tab_index(0)
                    .relative()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .w(px(POPUP_WIDTH))
                    .h(px(POPUP_HEIGHT))
                    .pl(px(9.))
                    .pr(px(5.))
                    .rounded(px(theme::R_TAG + 2.))
                    .border_1()
                    .border_color(color(theme::HOVER_LINE))
                    .bg(color(theme::CARD))
                    .shadow_sm()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_summarizer_menu(window, cx);
                    }))
                    .child(current.label())
                    .child(
                        Icon::new(IconName::ChevronsUpDown)
                            .size_3p5()
                            .text_color(color(theme::MUTED)),
                    )
                    .child(
                        canvas(move |b, _, _| bounds.set(Some(b)), |_, (), _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    .test_support(),
            )
            .children(missing.map(|why| {
                div()
                    .id("summarizer-missing")
                    .text_size(px(theme::FS_SMALL))
                    .text_color(color(theme::RED_TEXT))
                    .child(why)
                    .test_support()
            }))
            .into_any_element()
    }

    fn show(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    /// The toolbar: one icon over its name for each tab, the shown one highlighted.
    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .flex_none()
            .justify_center()
            .gap_1()
            .py_2()
            .border_b_1()
            .border_color(color(theme::LINE_2))
            .children(Tab::ALL.map(|tab| {
                let on = self.tab == tab;
                let tint = if on { theme::BLUE_TEXT } else { theme::MUTED };
                div()
                    .id(tab.label())
                    .role(Role::Tab)
                    .aria_label(tab.label())
                    .aria_selected(on)
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_0p5()
                    .w(px(68.))
                    .py_1()
                    .rounded(px(R_TAB))
                    .text_size(px(theme::FS_SMALL))
                    .text_color(color(tint))
                    .when(on, |d| d.bg(color(theme::SEL)))
                    .when(!on, |d| d.hover(|s| s.text_color(color(theme::FG))))
                    .on_click(cx.listener(move |this, _, _, cx| this.show(tab, cx)))
                    .child(Icon::new(tab.icon()).size_6())
                    .child(tab.label())
                    .test_support()
            }))
    }

    /// One row of the form, as in a macOS preferences pane: `section` right-aligned in a column on
    /// the left, `control` beside it, and `about` in small print under the control.
    fn row(
        section: &'static str,
        control: impl IntoElement,
        about: Option<&'static str>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .gap_3()
            .child(
                div()
                    .w(px(SECTION_WIDTH))
                    .flex_none()
                    .text_right()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(section),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap_1()
                    .child(control)
                    .children(about.map(|about| {
                        div()
                            .pl(px(ABOUT_INDENT))
                            .text_size(px(theme::FS_SMALL))
                            .text_color(color(theme::MUTED))
                            .child(about)
                    })),
            )
    }

    /// A checkbox with its `label`: a blue box with a check when `on`; the whole line toggles it.
    fn checkbox(
        id: &'static str,
        label: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        toggle: impl Fn(&mut Viewer, bool, &mut Context<Viewer>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .role(Role::CheckBox)
            .aria_label(label)
            .aria_toggled(if on { Toggled::True } else { Toggled::False })
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.viewer.update(cx, |viewer, cx| toggle(viewer, !on, cx));
            }))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(BOX))
                    .rounded(px(theme::R_TAG))
                    .border_1()
                    .text_size(px(theme::FS_SMALL))
                    .when_else(
                        on,
                        |d| {
                            let blue = color(theme::BLUE);
                            // Lit from above like a macOS checkbox: a lighter top, a darker edge
                            // and a soft shadow.
                            let top = Hsla {
                                l: (blue.l + 0.1).min(1.),
                                ..blue
                            };
                            let edge = Hsla {
                                l: blue.l * 0.8,
                                ..blue
                            };
                            d.bg(linear_gradient(
                                180.,
                                linear_color_stop(top, 0.),
                                linear_color_stop(blue, 1.),
                            ))
                            .border_color(edge)
                            .shadow_sm()
                            .text_color(gpui_kit::white())
                            .child("✓")
                        },
                        |d| {
                            d.bg(color(theme::CARD))
                                .border_color(color(theme::HOVER_LINE))
                                .shadow_sm()
                        },
                    ),
            )
            .child(label)
            .test_support()
    }
}

/// Corner radius of the highlight behind the shown tab.
const R_TAB: f32 = 8.;

impl Focusable for Settings {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let follow = self.viewer.read(cx).follows_newest();
        let summarizer = self.render_summarizer(cx);
        let pane = match self.tab {
            Tab::General => div()
                .flex()
                .flex_col()
                .gap_4()
                .child(Self::row(
                    "Sessions:",
                    Self::checkbox(
                        "follow",
                        "Follow newest session",
                        follow,
                        cx,
                        |viewer, on, cx| {
                            viewer.set_follow_newest(on, cx);
                        },
                    ),
                    Some("Switch to the newest session as soon as it starts."),
                ))
                .child(Self::row("Summaries:", summarizer, None)),
            Tab::About => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(Self::row(
                    "Ichnos:",
                    "Records what a coding agent does.",
                    None,
                ))
                .child(Self::row("Version:", env!("CARGO_PKG_VERSION"), None)),
        };
        div()
            .id("settings")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(|_: &CloseSettings, window, _| window.remove_window())
            .on_action(cx.listener(|this, _: &ShowGeneral, _, cx| this.show(Tab::General, cx)))
            .on_action(cx.listener(|this, _: &ShowAbout, _, cx| this.show(Tab::About, cx)))
            .on_action(cx.listener(|this, pick: &PickSummarizer, _, cx| {
                this.pick_summarizer(pick, cx);
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(color(theme::BG))
            .text_color(color(theme::FG))
            .text_size(px(theme::FS_BASE))
            .child(self.render_tabs(cx))
            .child(div().flex_1().px_6().py_5().child(pane))
    }
}
