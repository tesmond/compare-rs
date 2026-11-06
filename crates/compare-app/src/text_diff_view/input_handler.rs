//! Connects the platform text input system (typing, dead keys, and input
//! methods for composing characters) to the active side.
//!
//! The platform addresses text in UTF-16 offsets. This view presents the
//! cursor's line as the "document", which is all input methods need.

use std::ops::Range;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window, point};

use compare_core::{EditKind, Position};

use super::TextDiffView;
use super::side::Selection;
use crate::format;

/// Converts a byte offset in `text` to UTF-16 code units.
fn to_utf16(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    text[..byte].encode_utf16().count()
}

/// Converts a UTF-16 offset in `text` to a byte offset.
fn from_utf16(text: &str, units: usize) -> usize {
    let mut count = 0;
    for (index, ch) in text.char_indices() {
        if count >= units {
            return index;
        }
        count += ch.len_utf16();
    }
    text.len()
}

impl TextDiffView {
    /// The text of the active side's cursor line.
    fn input_line(&self) -> (usize, &str) {
        let state = self.side(self.active);
        let line = state.selection.head.line;
        (line, state.buffer.line(line))
    }

    /// The range an input method wants to replace: an explicit range of the
    /// cursor line, else the text being composed, else the selection.
    fn input_range(&self, range_utf16: Option<Range<usize>>) -> Range<Position> {
        let (line, text) = self.input_line();
        let state = self.side(self.active);
        if let Some(range) = range_utf16 {
            Position::new(line, from_utf16(text, range.start))
                ..Position::new(line, from_utf16(text, range.end))
        } else if let Some(marked) = &state.marked {
            Position::new(line, marked.start)..Position::new(line, marked.end)
        } else {
            state.selection.range()
        }
    }
}

impl EntityInputHandler for TextDiffView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let (_, text) = self.input_line();
        let start = from_utf16(text, range_utf16.start);
        let end = from_utf16(text, range_utf16.end).max(start);
        adjusted_range.replace(to_utf16(text, start)..to_utf16(text, end));
        Some(text[start..end].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let (line, text) = self.input_line();
        let selection = self.side(self.active).selection;
        let range = selection.range();
        let (start, end) = if range.start.line == line && range.end.line == line {
            (range.start.column, range.end.column)
        } else {
            (selection.head.column, selection.head.column)
        };
        Some(UTF16Selection {
            range: to_utf16(text, start)..to_utf16(text, end),
            reversed: selection.head < selection.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let (_, text) = self.input_line();
        let marked = self.side(self.active).marked.as_ref()?;
        Some(to_utf16(text, marked.start)..to_utf16(text, marked.end))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.side_mut(self.active).marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.input_range(range_utf16);
        let text = if text == "\r" { "\n" } else { text };
        self.edit(self.active, range, text, EditKind::Typing, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.input_range(range_utf16);
        let start = range.start.min(range.end);
        if !self.edit(self.active, range, new_text, EditKind::Typing, cx) {
            return;
        }
        let state = self.side_mut(self.active);
        state.marked = (!new_text.is_empty()).then(|| start.column..start.column + new_text.len());
        if let Some(selected) = new_selected_range_utf16 {
            let at = |units: usize| {
                Position::new(start.line, start.column + from_utf16(new_text, units))
            };
            state.selection =
                Selection { anchor: at(selected.start), head: at(selected.end), goal_column: None };
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (line, text) = self.input_line();
        let layouts = self.layouts.borrow();
        let layout = layouts.get(&(self.active, line))?;
        let offset = |units: usize| format::to_display_offset(text, from_utf16(text, units));
        let start = layout.position_for_index(offset(range_utf16.start))?;
        let end = layout.position_for_index(offset(range_utf16.end)).unwrap_or(start);
        let bottom = start.y + layout.line_height();
        Some(Bounds::from_corners(start, point(end.x.max(start.x), bottom)))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let (line, text) = self.input_line();
        let layouts = self.layouts.borrow();
        let index = match layouts.get(&(self.active, line))?.index_for_position(point) {
            Ok(index) | Err(index) => index,
        };
        Some(to_utf16(text, format::from_display_offset(text, index)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_utf16_offsets() {
        let text = "a😀b";
        assert_eq!(to_utf16(text, 5), 3);
        assert_eq!(from_utf16(text, 3), 5);
        assert_eq!(from_utf16(text, 99), text.len());
        assert_eq!(to_utf16(text, 2), 1);
    }
}
