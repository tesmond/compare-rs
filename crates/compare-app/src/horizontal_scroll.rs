//! Horizontal scrolling shared by the line-based views.

use gpui::{Pixels, ScrollWheelEvent, px};

use crate::theme;

/// Approximate advance width of one character of the code font, as a
/// fraction of the font size (Menlo's advance is 0.602 em).
const CHAR_WIDTH_EM: f32 = 0.602;

/// Horizontal offset applied to the text of every row, so long lines can be
/// scrolled sideways while the line-number gutters stay put.
#[derive(Clone, Copy, Debug, Default)]
pub struct HorizontalScroll {
    offset: Pixels,
    max_offset: Pixels,
}

impl HorizontalScroll {
    /// Allows scrolling far enough to reveal `max_columns` characters.
    pub fn new(max_columns: usize) -> Self {
        Self { offset: px(0.0), max_offset: Self::offset_for(max_columns) }
    }

    /// Width of one character of the code font.
    pub fn char_width() -> Pixels {
        theme::code_text_size() * CHAR_WIDTH_EM
    }

    fn offset_for(max_columns: usize) -> Pixels {
        // Keep a few columns visible at the far end.
        Self::char_width() * max_columns.saturating_sub(8) as f32
    }

    /// Allows scrolling to at least `columns` characters, for lines that
    /// grew while editing.
    pub fn grow(&mut self, columns: usize) {
        self.max_offset = self.max_offset.max(Self::offset_for(columns));
    }

    /// Scrolls just enough to show display column `column` in a text area
    /// `width` wide. Returns true if the offset changed.
    pub fn reveal(&mut self, column: usize, width: Pixels) -> bool {
        let char_width = Self::char_width();
        let x = char_width * column as f32;
        let margin = char_width * 4.0;
        let old = self.offset;
        if x < self.offset + margin {
            self.offset = (x - margin).max(px(0.0));
        } else if width > margin * 2.0 && x > self.offset + width - margin {
            self.offset = x - width + margin;
        }
        self.max_offset = self.max_offset.max(self.offset);
        self.offset != old
    }

    /// Scrolls back to the left edge.
    pub fn reset(&mut self) {
        self.offset = px(0.0);
    }

    /// The current offset, to be applied as a negative left margin.
    pub fn offset(&self) -> Pixels {
        self.offset
    }

    /// Applies the horizontal part of a scroll-wheel event. Returns true if
    /// the event was mostly horizontal and therefore consumed.
    pub fn handle(&mut self, event: &ScrollWheelEvent) -> bool {
        let delta = event.delta.pixel_delta(theme::row_height());
        if delta.x.abs() <= delta.y.abs() {
            return false;
        }
        self.offset = (self.offset - delta.x).clamp(px(0.0), self.max_offset.max(px(0.0)));
        true
    }
}

#[cfg(test)]
mod tests {
    use gpui::{ScrollDelta, point};

    use super::*;

    fn wheel(x: f32, y: f32) -> ScrollWheelEvent {
        ScrollWheelEvent { delta: ScrollDelta::Pixels(point(px(x), px(y))), ..Default::default() }
    }

    #[test]
    fn horizontal_wheel_moves_and_clamps() {
        let mut scroll = HorizontalScroll::new(100);
        assert!(scroll.handle(&wheel(-50.0, 0.0)));
        assert_eq!(scroll.offset(), px(50.0));
        assert!(scroll.handle(&wheel(1000.0, 0.0)));
        assert_eq!(scroll.offset(), px(0.0));
        assert!(scroll.handle(&wheel(-100_000.0, 0.0)));
        assert!(scroll.offset() > px(0.0) && scroll.offset() < px(1000.0));
    }

    #[test]
    fn vertical_wheel_is_ignored() {
        let mut scroll = HorizontalScroll::new(100);
        assert!(!scroll.handle(&wheel(-5.0, -40.0)));
        assert_eq!(scroll.offset(), px(0.0));
    }

    #[test]
    fn reveal_keeps_the_cursor_in_view() {
        let mut scroll = HorizontalScroll::new(0);
        let width = HorizontalScroll::char_width() * 40.0;
        assert!(!scroll.reveal(10, width));
        assert!(scroll.reveal(100, width));
        let offset = scroll.offset();
        assert!(offset > px(0.0));
        assert!(!scroll.reveal(95, width));
        assert!(scroll.reveal(0, width));
        assert_eq!(scroll.offset(), px(0.0));
    }

    #[test]
    fn short_lines_do_not_scroll() {
        let mut scroll = HorizontalScroll::new(4);
        scroll.handle(&wheel(-50.0, 0.0));
        assert_eq!(scroll.offset(), px(0.0));
    }
}
