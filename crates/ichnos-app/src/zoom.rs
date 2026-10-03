//! Scaling a subtree: GPUI has no transforms, so [`Zoom`] lays out and paints its child with a
//! scaled rem size. Everything sized in rems under it (boxes, gaps, text, radii) scales with it;
//! pixel sizes don't.

use gpui_kit::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

/// `child` drawn at `scale` times its rem-based size.
pub fn zoom(scale: f32, child: impl IntoElement) -> Zoom {
    Zoom {
        scale,
        child: child.into_any_element(),
    }
}

/// An element that draws its child with the rem size multiplied by a scale.
pub struct Zoom {
    scale: f32,
    child: AnyElement,
}

impl Zoom {
    /// The rem size the child is drawn with.
    fn rem_size(&self, window: &Window) -> Pixels {
        window.rem_size() * self.scale
    }
}

impl Element for Zoom {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let rem_size = self.rem_size(window);
        let child = &mut self.child;
        let layout_id =
            window.with_rem_size(Some(rem_size), |window| child.request_layout(window, cx));
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let rem_size = self.rem_size(window);
        let child = &mut self.child;
        window.with_rem_size(Some(rem_size), |window| child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        (): &mut (),
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let rem_size = self.rem_size(window);
        let child = &mut self.child;
        window.with_rem_size(Some(rem_size), |window| child.paint(window, cx));
    }
}

impl IntoElement for Zoom {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
