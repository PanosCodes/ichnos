//! How the graph is panned and zoomed.

/// Pointer travel, in pixels, beyond which a press counts as a drag rather than a click.
const DRAG_THRESHOLD: f32 = 3.;

/// The graph's smallest and largest zoom.
pub const ZOOM_RANGE: (f32, f32) = (0.25, 3.);

/// How the graph is moved from its centered position and zoomed, and the drag in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pan {
    /// In screen pixels.
    offset: (f32, f32),
    zoom: f32,
    /// Where the pointer and the offset were when the drag started.
    drag: Option<((f32, f32), (f32, f32))>,
    /// Whether the last press moved the graph, so its release isn't a click on a card.
    dragged: bool,
}

impl Default for Pan {
    fn default() -> Self {
        Self {
            offset: (0., 0.),
            zoom: 1.,
            drag: None,
            dragged: false,
        }
    }
}

impl Pan {
    /// The zoom: 1 is actual size.
    #[must_use]
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Zoom by `factor` (above 1 zooms in) within [`ZOOM_RANGE`], keeping what's at the center
    /// of the pane in place.
    pub fn zoom_by(&mut self, factor: f32) {
        let zoom = (self.zoom * factor).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        let ratio = zoom / self.zoom;
        self.offset = (self.offset.0 * ratio, self.offset.1 * ratio);
        self.zoom = zoom;
    }

    /// Back to actual size, keeping the center in place.
    pub fn reset_zoom(&mut self) {
        self.zoom_by(1. / self.zoom);
    }

    /// The offset from the centered position.
    #[must_use]
    pub fn offset(&self) -> (f32, f32) {
        self.offset
    }

    /// Whether a drag is in progress.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Whether the last press moved the graph rather than clicked.
    #[must_use]
    pub fn was_drag(&self) -> bool {
        self.dragged
    }

    /// The pointer went down at `pointer`.
    pub fn start(&mut self, pointer: (f32, f32)) {
        self.drag = Some((pointer, self.offset));
        self.dragged = false;
    }

    /// The pointer moved to `pointer`; `true` when the graph moved.
    pub fn drag_to(&mut self, pointer: (f32, f32)) -> bool {
        let Some((from, offset)) = self.drag else {
            return false;
        };
        let (dx, dy) = (pointer.0 - from.0, pointer.1 - from.1);
        if !self.dragged && dx.hypot(dy) < DRAG_THRESHOLD {
            return false;
        }
        self.dragged = true;
        self.offset = (offset.0 + dx, offset.1 + dy);
        true
    }

    /// The pointer went up.
    pub fn end(&mut self) {
        self.drag = None;
    }

    /// Move the graph by a scroll of `delta`.
    pub fn scroll(&mut self, delta: (f32, f32)) {
        self.offset = (self.offset.0 + delta.0, self.offset.1 + delta.1);
    }

    /// Back to the centered position.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
