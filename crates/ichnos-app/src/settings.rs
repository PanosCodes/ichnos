//! The settings window: a small fixed-size window laid out like a macOS preferences pane, with a
//! row of icon tabs on top and a form of right-aligned section names beside their controls.

use accesskit::{Role, Toggled};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable,
    FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowOptions, actions, div, linear_color_stop, linear_gradient, px, size,
};

use crate::theme::{self, color};
use crate::viewer::Viewer;

/// Width of the column of section names, which are right-aligned in it.
const SECTION_WIDTH: f32 = 84.;
/// Where a control's small print starts: under its label, past the checkbox.
const ABOUT_INDENT: f32 = 24.;
/// Side of the checkbox.
const BOX: f32 = 16.;

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
    let bounds = Bounds::centered(None, size(px(520.), px(200.)), cx);
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

/// The settings window's root view.
pub struct Settings {
    focus: FocusHandle,
    viewer: Entity<Viewer>,
    tab: Tab,
}

impl Settings {
    pub(crate) fn new(viewer: Entity<Viewer>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The checkbox follows the viewer, which can also change on its own.
        cx.observe(&viewer, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            focus,
            viewer,
            tab: Tab::General,
        }
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
        let pane = match self.tab {
            Tab::General => div().child(Self::row(
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
            )),
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
