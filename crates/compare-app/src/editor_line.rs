//! One editable line of text: styled text plus a cursor.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, Hsla, InspectorElementId, IntoElement,
    LayoutId, Pixels, StyledText, TextLayout, Window, fill, px, size,
};

/// Called after painting with the line's text layout, so the owning view
/// can map mouse positions to text offsets and back.
pub type LayoutSink = Box<dyn FnOnce(TextLayout, &mut App)>;

/// Width of the text cursor.
const CURSOR_WIDTH: f32 = 2.0;

/// Paints styled text (wrapped or not, as the inherited text style says)
/// and, optionally, a cursor at a byte offset of that text.
pub struct EditorLine {
    text: StyledText,
    /// Byte offset into the text, and the cursor color.
    cursor: Option<(usize, Hsla)>,
    on_layout: Option<LayoutSink>,
}

impl EditorLine {
    pub fn new(text: StyledText) -> Self {
        Self { text, cursor: None, on_layout: None }
    }

    /// Draws a cursor before byte `offset` of the text.
    pub fn cursor(mut self, offset: usize, color: Hsla) -> Self {
        self.cursor = Some((offset, color));
        self
    }

    /// Hands the text layout to `sink` once it has been painted.
    pub fn on_layout(mut self, sink: LayoutSink) -> Self {
        self.on_layout = Some(sink);
        self
    }
}

impl IntoElement for EditorLine {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorLine {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.text.prepaint(id, inspector_id, bounds, request_layout, window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text.paint(id, inspector_id, bounds, request_layout, prepaint, window, cx);
        let layout = self.text.layout().clone();
        if let Some((offset, color)) = self.cursor
            && let Some(origin) = layout.position_for_index(offset)
        {
            let cursor = Bounds::new(origin, size(px(CURSOR_WIDTH), layout.line_height()));
            window.paint_quad(fill(cursor, color));
        }
        if let Some(sink) = self.on_layout.take() {
            sink(layout, cx);
        }
    }
}
