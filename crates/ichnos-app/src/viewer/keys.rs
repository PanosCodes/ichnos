//! The viewer's actions, their key bindings, the handlers of the moving ones and the app menu.

use gpui_kit::{App, Context, KeyBinding, Menu, MenuItem, Window, actions};

use super::{CONTEXT, SHORTCUTS, Viewer};

actions!(
    ichnos,
    [
        /// Select the next row.
        MoveDown,
        /// Select the previous row.
        MoveUp,
        /// Expand or collapse the selected row.
        ToggleRow,
        /// Jump to the next failed call.
        NextError,
        /// Start or pause replay.
        TogglePlay,
        /// Move the replay cursor back one moment.
        StepBack,
        /// Move the replay cursor forward one moment.
        StepForward,
        /// Show or hide the right-hand panel.
        TogglePanel,
        /// Open or close the keyboard shortcuts overlay.
        ToggleHelp,
        /// Show the tree.
        ShowTree,
        /// Show the graph.
        ShowGraph,
        /// Show the table.
        ShowTable,
        /// Focus the filter box.
        FocusFilter,
        /// Open or close the session switcher.
        ToggleSwitcher,
        /// Open the settings window.
        OpenSettings,
        /// Close the open overlay.
        Dismiss,
        /// Zoom the graph in.
        ZoomIn,
        /// Zoom the graph out.
        ZoomOut,
        /// Show the graph at actual size.
        ZoomReset,
        /// Quit the app.
        Quit,
    ]
);

/// Register the viewer's keyboard shortcuts.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("j", MoveDown, Some(SHORTCUTS)),
        KeyBinding::new("down", MoveDown, Some(SHORTCUTS)),
        KeyBinding::new("k", MoveUp, Some(SHORTCUTS)),
        KeyBinding::new("up", MoveUp, Some(SHORTCUTS)),
        KeyBinding::new("enter", ToggleRow, Some(SHORTCUTS)),
        KeyBinding::new("e", NextError, Some(SHORTCUTS)),
        KeyBinding::new("space", TogglePlay, Some(SHORTCUTS)),
        KeyBinding::new("left", StepBack, Some(SHORTCUTS)),
        KeyBinding::new("right", StepForward, Some(SHORTCUTS)),
        KeyBinding::new("cmd-\\", TogglePanel, Some(CONTEXT)),
        KeyBinding::new("?", ToggleHelp, Some(SHORTCUTS)),
        KeyBinding::new("shift-/", ToggleHelp, Some(SHORTCUTS)),
        KeyBinding::new("t", ShowTree, Some(SHORTCUTS)),
        KeyBinding::new("g", ShowGraph, Some(SHORTCUTS)),
        KeyBinding::new("b", ShowTable, Some(SHORTCUTS)),
        KeyBinding::new("/", FocusFilter, Some(SHORTCUTS)),
        KeyBinding::new("cmd-,", OpenSettings, Some(CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(SHORTCUTS)),
        KeyBinding::new("cmd-k", ToggleSwitcher, Some(CONTEXT)),
        KeyBinding::new("ctrl-k", ToggleSwitcher, Some(CONTEXT)),
        KeyBinding::new("cmd-=", ZoomIn, Some(SHORTCUTS)),
        KeyBinding::new("cmd-+", ZoomIn, Some(SHORTCUTS)),
        KeyBinding::new("cmd--", ZoomOut, Some(SHORTCUTS)),
        KeyBinding::new("cmd-0", ZoomReset, Some(SHORTCUTS)),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    crate::settings::bind_keys(cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

/// The app menu: "Settings…" opens the settings screen, and "Quit" leaves.
pub fn set_menus(cx: &mut App) {
    cx.set_menus([Menu {
        name: "Ichnos".into(),
        items: vec![
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Quit Ichnos", Quit),
        ],
        disabled: false,
    }]);
}

impl Viewer {
    pub(super) fn on_dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.switcher = None;
        self.ask = None;
        self.help = false;
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn on_move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        let order = self.order();
        self.state.move_selection(&order, 1);
        self.scroll_to_pending();
        cx.notify();
    }

    pub(super) fn on_move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        let order = self.order();
        self.state.move_selection(&order, -1);
        self.scroll_to_pending();
        cx.notify();
    }

    pub(super) fn on_toggle_row(&mut self, _: &ToggleRow, _: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.state.selected().and_then(|key| self.model.find(key)) else {
            return;
        };
        self.state.toggle(&self.model, id);
        self.refresh();
        cx.notify();
    }

    pub(super) fn on_next_error(&mut self, _: &NextError, _: &mut Window, cx: &mut Context<Self>) {
        if self.state.next_error(&self.model).is_some() {
            self.refresh();
            self.scroll_to_pending();
            cx.notify();
        }
    }
}
