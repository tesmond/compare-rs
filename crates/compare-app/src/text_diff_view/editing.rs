//! Editing commands: typing, deleting, clipboard, undo, and moving the cursor.

use std::ops::Range;
use std::time::Instant;

use gpui::{
    ClipboardItem, Context, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Window,
    point, px,
};

use compare_core::{DiffSide, Edit, EditBuffer, EditKind, Position};

use super::motion;
use super::render::GUTTER_WIDTH;
use super::side::{Selection, SideState};
use super::{REDIFF_DELAY, TextDiffView, display_width};
use crate::format;
use crate::theme;

/// A way of moving the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
}

/// Spaces per indent level when the file does not indent with tabs.
const INDENT_WIDTH: usize = 4;

impl TextDiffView {
    pub(super) fn side(&self, side: DiffSide) -> &SideState {
        &self.sides[side.index()]
    }

    pub(super) fn side_mut(&mut self, side: DiffSide) -> &mut SideState {
        &mut self.sides[side.index()]
    }

    fn active_side(&self) -> &SideState {
        self.side(self.active)
    }

    /// Replaces `range` of `side` with `text` and updates the syntax state
    /// and diff, without touching the undo history or selection.
    pub(super) fn apply_edit(
        &mut self,
        side: DiffSide,
        range: Range<Position>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Edit {
        let state = self.side_mut(side);
        let (start, end) = (state.buffer.clamp(range.start), state.buffer.clamp(range.end));
        let range = if end < start { end..start } else { start..end };
        let old_text = state.buffer.text_in(range.clone());
        let change = state.buffer.replace(range.clone(), text);
        state.syntax.apply_edit(change, state.buffer.lines());
        let new_lines = &state.buffer.lines()[change.first_line..][..change.new_count];
        let columns = new_lines.iter().map(|line| display_width(line)).max().unwrap_or(0);
        self.horizontal.grow(columns);

        let [left, right] = &self.sides;
        let splice = self.diff.apply_edit(
            side,
            change,
            left.buffer.lines(),
            right.buffer.lines(),
            &self.options,
        );
        self.list.splice(splice.old, splice.new_len);
        self.identical = false;
        self.schedule_rediff(REDIFF_DELAY, cx);
        Edit { start: range.start, old_text, new_text: text.to_owned() }
    }

    /// Replaces `range` of `side` with `text` as one undoable edit and puts
    /// the cursor after it. Returns false if the side is read-only.
    pub(super) fn edit(
        &mut self,
        side: DiffSide,
        range: Range<Position>,
        text: &str,
        kind: EditKind,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.side(side).read_only.is_some() {
            return false;
        }
        let before = self.side(side).selection;
        let edit = self.apply_edit(side, range, text, cx);
        let after = Selection::caret(edit.new_range().end);
        let state = self.side_mut(side);
        state.selection = after;
        state.marked = None;
        state.history.record(edit, kind, before, after, Instant::now());
        self.reveal_cursor();
        cx.notify();
        true
    }

    /// Replaces the active side's selection with `text`.
    pub(super) fn replace_selection(&mut self, text: &str, kind: EditKind, cx: &mut Context<Self>) {
        let range = self.active_side().selection.range();
        self.edit(self.active, range, text, kind, cx);
    }

    pub(super) fn undo(&mut self, cx: &mut Context<Self>) {
        let side = self.active;
        if self.side(side).read_only.is_some() {
            return;
        }
        let Some(step) = self.side_mut(side).history.undo() else {
            return;
        };
        for edit in step.edits.iter().rev() {
            self.apply_edit(side, edit.new_range(), &edit.old_text, cx);
        }
        self.side_mut(side).selection = step.selection_before;
        self.reveal_cursor();
        cx.notify();
    }

    pub(super) fn redo(&mut self, cx: &mut Context<Self>) {
        let side = self.active;
        if self.side(side).read_only.is_some() {
            return;
        }
        let Some(step) = self.side_mut(side).history.redo() else {
            return;
        };
        for edit in &step.edits {
            self.apply_edit(side, edit.old_range(), &edit.new_text, cx);
        }
        self.side_mut(side).selection = step.selection_after;
        self.reveal_cursor();
        cx.notify();
    }

    // ─── Cursor movement ────────────────────────────────────────────────────

    /// Puts a plain cursor at `position` on the active side.
    pub(super) fn set_cursor(&mut self, position: Position) {
        let state = self.side_mut(self.active);
        state.selection = Selection::caret(state.buffer.clamp(position));
        state.marked = None;
        state.history.break_coalescing();
    }

    /// Moves the cursor, or extends the selection when `select` is true.
    pub(super) fn move_cursor(&mut self, motion: Motion, select: bool, cx: &mut Context<Self>) {
        let selection = self.active_side().selection;
        let collapse_to =
            |position: Position| (!select && !selection.is_empty()).then_some(position);
        let (head, goal_column) = match motion {
            Motion::Left => match collapse_to(selection.range().start) {
                Some(start) => (start, None),
                None => (motion::prev_char(&self.active_side().buffer, selection.head), None),
            },
            Motion::Right => match collapse_to(selection.range().end) {
                Some(end) => (end, None),
                None => (motion::next_char(&self.active_side().buffer, selection.head), None),
            },
            Motion::Up | Motion::Down => self.vertical_target(selection, motion == Motion::Down, 1),
            Motion::PageUp | Motion::PageDown => {
                let lines = self.page_lines();
                self.vertical_target(selection, motion == Motion::PageDown, lines)
            }
            other => (self.target(other, selection.head), None),
        };
        let state = self.side_mut(self.active);
        let anchor = if select { selection.anchor } else { head };
        state.selection = Selection { anchor, head, goal_column };
        state.marked = None;
        state.history.break_coalescing();
        self.reveal_cursor();
        cx.notify();
    }

    /// Where a non-vertical motion takes the cursor from `head`.
    fn target(&self, motion: Motion, head: Position) -> Position {
        let buffer = &self.active_side().buffer;
        match motion {
            Motion::WordLeft => motion::prev_word(buffer, head),
            Motion::WordRight => motion::next_word(buffer, head),
            Motion::LineStart => motion::line_start(buffer, head),
            Motion::LineEnd => Position::new(head.line, buffer.line(head.line).len()),
            Motion::DocStart => Position::default(),
            Motion::DocEnd => buffer.end(),
            Motion::Left => motion::prev_char(buffer, head),
            Motion::Right => motion::next_char(buffer, head),
            Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown => head,
        }
    }

    /// Where moving `lines` lines up or down takes the cursor, keeping the
    /// goal column. Within a wrapped line, moves by visual line first.
    fn vertical_target(
        &self,
        selection: Selection,
        down: bool,
        lines: usize,
    ) -> (Position, Option<usize>) {
        let buffer = &self.active_side().buffer;
        let head = selection.head;
        let goal = selection
            .goal_column
            .unwrap_or_else(|| format::display_column(buffer.line(head.line), head.column));
        if self.wrap
            && lines == 1
            && let Some(position) = self.visual_line_target(head, down)
        {
            return (position, Some(goal));
        }
        let target = if down { head.line.checked_add(lines) } else { head.line.checked_sub(lines) };
        match target {
            Some(line) if line < buffer.len() => {
                (Position::new(line, format::offset_at_column(buffer.line(line), goal)), Some(goal))
            }
            _ if down => (buffer.end(), Some(goal)),
            _ => (Position::default(), Some(goal)),
        }
    }

    /// The position one visual line up or down within the same wrapped
    /// line, if there is one.
    fn visual_line_target(&self, head: Position, down: bool) -> Option<Position> {
        let layouts = self.layouts.borrow();
        let layout = layouts.get(&(self.active, head.line))?;
        let text = self.active_side().buffer.line(head.line);
        let origin = layout.position_for_index(format::to_display_offset(text, head.column))?;
        let line_height = layout.line_height();
        let bounds = layout.bounds();
        let y = if down { origin.y + line_height * 1.5 } else { origin.y - line_height * 0.5 };
        if y < bounds.top() || y > bounds.bottom() {
            return None;
        }
        let index = match layout.index_for_position(point(origin.x, y)) {
            Ok(index) | Err(index) => index,
        };
        Some(Position::new(head.line, format::from_display_offset(text, index)))
    }

    /// How many lines a page up or down moves.
    fn page_lines(&self) -> usize {
        let height = self.list.viewport_bounds().size.height;
        ((height / theme::row_height()) as usize).saturating_sub(2).max(1)
    }

    /// Scrolls so the active side's cursor is visible.
    pub(super) fn reveal_cursor(&mut self) {
        let head = self.active_side().selection.head;
        let row = self.diff.row_of_line(self.active, head.line);
        self.list.scroll_to_reveal_item(row);
        if !self.wrap {
            let column =
                format::display_column(self.active_side().buffer.line(head.line), head.column);
            self.horizontal.reveal(column, self.text_area_width());
        }
    }

    /// Width of one side's text area (excluding the line-number gutter).
    fn text_area_width(&self) -> Pixels {
        let width = self.list.viewport_bounds().size.width;
        ((width - px(4.0)) / 2.0 - px(GUTTER_WIDTH) - px(8.0)).max(px(0.0))
    }

    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        let state = self.side_mut(self.active);
        state.selection =
            Selection { anchor: Position::default(), head: state.buffer.end(), goal_column: None };
        cx.notify();
    }

    pub(super) fn collapse_selection(&mut self, cx: &mut Context<Self>) {
        let head = self.active_side().selection.head;
        self.set_cursor(head);
        cx.notify();
    }

    // ─── Deleting and inserting ─────────────────────────────────────────────

    /// Deletes the selection, or from the cursor to where `to` moves it.
    pub(super) fn delete(
        &mut self,
        to: fn(&EditBuffer, Position) -> Position,
        kind: EditKind,
        cx: &mut Context<Self>,
    ) {
        let selection = self.active_side().selection;
        if !selection.is_empty() {
            self.replace_selection("", EditKind::Other, cx);
            return;
        }
        let target = to(&self.active_side().buffer, selection.head);
        if target != selection.head {
            let range = target.min(selection.head)..target.max(selection.head);
            self.edit(self.active, range, "", kind, cx);
        }
    }

    /// Breaks the line, carrying the current indentation onto the new line.
    pub(super) fn newline(&mut self, cx: &mut Context<Self>) {
        let start = self.active_side().selection.range().start;
        let line = self.active_side().buffer.line(start.line);
        let indent = motion::indentation(&line[..start.column]);
        let text = format!("\n{indent}");
        self.replace_selection(&text, EditKind::Other, cx);
    }

    /// Inserts a tab, or spaces to the next tab stop if the file indents
    /// with spaces.
    pub(super) fn indent(&mut self, cx: &mut Context<Self>) {
        let state = self.active_side();
        let uses_tabs = state.buffer.lines().iter().any(|line| line.starts_with('\t'));
        let text = if uses_tabs {
            "\t".to_owned()
        } else {
            let head = state.selection.range().start;
            let column = format::display_column(state.buffer.line(head.line), head.column);
            " ".repeat(INDENT_WIDTH - column % INDENT_WIDTH)
        };
        self.replace_selection(&text, EditKind::Typing, cx);
    }

    /// The selected text, or the whole cursor line when nothing is selected.
    fn selected_or_line(&self) -> (String, Range<Position>) {
        let state = self.active_side();
        let selection = state.selection;
        if !selection.is_empty() {
            return (state.buffer.text_in(selection.range()), selection.range());
        }
        let line = selection.head.line;
        let text = format!("{}\n", state.buffer.line(line));
        let range = if line + 1 < state.buffer.len() {
            Position::new(line, 0)..Position::new(line + 1, 0)
        } else if line > 0 {
            Position::new(line - 1, state.buffer.line(line - 1).len())
                ..Position::new(line, state.buffer.line(line).len())
        } else {
            Position::new(line, 0)..Position::new(line, state.buffer.line(line).len())
        };
        (text, range)
    }

    pub(super) fn copy(&mut self, cx: &mut Context<Self>) {
        let (text, _) = self.selected_or_line();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    pub(super) fn cut(&mut self, cx: &mut Context<Self>) {
        if self.active_side().read_only.is_some() {
            self.copy(cx);
            return;
        }
        let (text, range) = self.selected_or_line();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.edit(self.active, range, "", EditKind::Other, cx);
    }

    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.replace_selection(&text, EditKind::Other, cx);
    }

    // ─── Mouse ──────────────────────────────────────────────────────────────

    /// Places the cursor (or selects a word or line) where `side` of `row`
    /// was clicked.
    pub(super) fn mouse_down(
        &mut self,
        side: DiffSide,
        row: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        let position = self.position_at(side, row, event.position);
        let extend = event.modifiers.shift && side == self.active;
        self.active = side;
        self.dragging = true;
        let state = self.side_mut(side);
        state.marked = None;
        state.history.break_coalescing();
        state.selection = match event.click_count {
            2 => {
                let (start, end) = motion::word_at(&state.buffer, position);
                Selection { anchor: start, head: end, goal_column: None }
            }
            count if count >= 3 => {
                let line = position.line;
                let end = if line + 1 < state.buffer.len() {
                    Position::new(line + 1, 0)
                } else {
                    Position::new(line, state.buffer.line(line).len())
                };
                Selection { anchor: Position::new(line, 0), head: end, goal_column: None }
            }
            _ if extend => Selection { head: position, goal_column: None, ..state.selection },
            _ => Selection::caret(position),
        };
        cx.notify();
    }

    /// Extends the selection while dragging over `side` of `row`.
    pub(super) fn mouse_drag(
        &mut self,
        side: DiffSide,
        row: usize,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.dragging || side != self.active || event.pressed_button != Some(MouseButton::Left)
        {
            return;
        }
        let position = self.position_at(side, row, event.position);
        let state = self.side_mut(side);
        if state.selection.head != position {
            state.selection.head = position;
            state.selection.goal_column = None;
            cx.notify();
        }
    }

    pub(super) fn mouse_up(&mut self) {
        self.dragging = false;
    }

    /// The text position under `point` in `side` of `row`. A filler row
    /// maps to the end of the line above it.
    fn position_at(&self, side: DiffSide, row: usize, point: Point<Pixels>) -> Position {
        let buffer = &self.side(side).buffer;
        let Some(line) = self.diff.rows().get(row).and_then(|&r| side.line_of(r)) else {
            let before = self.diff.lines_before_row(side, row);
            return match before.checked_sub(1) {
                Some(line) => Position::new(line, buffer.line(line).len()),
                None => Position::default(),
            };
        };
        let line = line as usize;
        let text = buffer.line(line);
        let index = self.layouts.borrow().get(&(side, line)).map_or(0, |layout| {
            match layout.index_for_position(point) {
                Ok(index) | Err(index) => index,
            }
        });
        Position::new(line, format::from_display_offset(text, index))
    }
}
