//! The right-hand panel's width and the drag that changes it.

/// The right-hand panel's default width, in pixels.
pub const PANEL_DEFAULT: f32 = 380.;

/// The right-hand panel's narrowest and widest, in pixels.
pub const PANEL_RANGE: (f32, f32) = (260., 720.);

/// The width, in pixels, the main pane keeps however wide the panel is dragged.
pub const MAIN_MIN: f32 = 320.;

/// The width, in pixels, of the divider between the main pane and the panel.
pub const HANDLE_WIDTH: f32 = 5.;

/// The main pane width, in pixels, below which tree rows drop their timeline bar so the label
/// keeps its room.
pub const BARS_MIN_MAIN: f32 = 640.;

/// The width of the right-hand panel, whether it shows, and the resize drag in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelWidth {
    pub(super) width: f32,
    pub(super) open: bool,
    /// Where the pointer and the width were when the drag started.
    pub(super) drag: Option<(f32, f32)>,
}

impl Default for PanelWidth {
    fn default() -> Self {
        Self {
            width: PANEL_DEFAULT,
            open: true,
            drag: None,
        }
    }
}

impl PanelWidth {
    /// The width, in pixels.
    #[must_use]
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Whether the panel shows.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Show the panel if hidden, hide it if shown; its width is kept for next time.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.drag = None;
    }

    /// The main pane's width in a window `window` pixels wide.
    #[must_use]
    pub fn main_width(&self, window: f32) -> f32 {
        if self.open {
            window - self.width - HANDLE_WIDTH
        } else {
            window
        }
    }

    /// Whether a drag is in progress.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Back to the default width.
    pub fn reset(&mut self) {
        self.width = PANEL_DEFAULT;
    }

    /// The pointer went down at `x`.
    pub fn start(&mut self, x: f32) {
        self.drag = Some((x, self.width));
    }

    /// The pointer moved to `x` in a window `window` pixels wide; `true` when the width changed.
    /// Dragging left widens the panel.
    pub fn drag_to(&mut self, x: f32, window: f32) -> bool {
        let Some((from, width)) = self.drag else {
            return false;
        };
        let most = (window - MAIN_MIN).clamp(PANEL_RANGE.0, PANEL_RANGE.1);
        let next = (width + from - x).clamp(PANEL_RANGE.0, most);
        let moved = (next - self.width).abs() > f32::EPSILON;
        self.width = next;
        moved
    }

    /// The pointer went up.
    pub fn end(&mut self) {
        self.drag = None;
    }
}
