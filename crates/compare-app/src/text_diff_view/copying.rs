//! Copying a changed block, or a single line, from one side to the other.

use std::ops::Range;

use gpui::Context;

use compare_core::{DiffSide, EditBuffer, EditKind, Position, RowKind};

use super::TextDiffView;

/// Which context-menu edits are available.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct EditMenuState {
    pub insert_line: bool,
    /// Copying a line or section into the left side.
    pub copy_left: bool,
    /// Copying a line or section into the right side.
    pub copy_right: bool,
}

/// The text edit that turns `lines` of `buffer` into `new_lines`: the range
/// to replace and its replacement.
fn line_replacement(
    buffer: &EditBuffer,
    lines: Range<usize>,
    new_lines: &[String],
) -> Option<(Range<Position>, String)> {
    let joined = new_lines.join("\n");
    let line_end = |line: usize| Position::new(line, buffer.line(line).len());
    match (lines.is_empty(), new_lines.is_empty()) {
        (true, true) => None,
        // Replace whole lines, keeping the line break after the last one.
        (false, false) => Some((Position::new(lines.start, 0)..line_end(lines.end - 1), joined)),
        // Insert new lines before line `lines.start`.
        (true, false) if lines.start < buffer.len() => {
            let at = Position::new(lines.start, 0);
            Some((at..at, format!("{joined}\n")))
        }
        (true, false) => {
            let end = buffer.end();
            Some((end..end, format!("\n{joined}")))
        }
        // Delete whole lines, including one line break.
        (false, true) if lines.end < buffer.len() => {
            Some((Position::new(lines.start, 0)..Position::new(lines.end, 0), String::new()))
        }
        (false, true) if lines.start > 0 => {
            Some((line_end(lines.start - 1)..buffer.end(), String::new()))
        }
        (false, true) => Some((Position::default()..buffer.end(), String::new())),
    }
}

impl TextDiffView {
    /// The rows of the changed region containing `row`, if it is changed.
    pub(super) fn change_rows_around(&self, row: usize) -> Option<Range<usize>> {
        let rows = self.diff.rows();
        if rows.get(row)?.kind == RowKind::Equal {
            return None;
        }
        let start = rows[..row].iter().rposition(|r| r.kind == RowKind::Equal).map_or(0, |i| i + 1);
        let end = rows[row..]
            .iter()
            .position(|r| r.kind == RowKind::Equal)
            .map_or(rows.len(), |i| row + i);
        Some(start..end)
    }

    /// Returns true if `row` starts a changed region.
    pub(super) fn starts_change(&self, row: usize) -> bool {
        let rows = self.diff.rows();
        rows[row].kind != RowKind::Equal && (row == 0 || rows[row - 1].kind == RowKind::Equal)
    }

    /// Makes the other side's lines in `rows` match `from`'s lines there.
    pub(super) fn copy_rows(&mut self, rows: Range<usize>, from: DiffSide, cx: &mut Context<Self>) {
        let to = from.other();
        if self.side(to).read_only.is_some() {
            return;
        }
        let lines = |side: DiffSide| {
            self.diff.lines_before_row(side, rows.start)..self.diff.lines_before_row(side, rows.end)
        };
        let (source, target) = (lines(from), lines(to));
        let new_lines = self.side(from).buffer.lines()[source].to_vec();
        let Some((range, text)) = line_replacement(&self.side(to).buffer, target, &new_lines)
        else {
            return;
        };
        let selection = self.side(to).selection;
        self.edit(to, range, &text, EditKind::Other, cx);
        // Copying should not move the cursor on the side being written to.
        let state = self.side_mut(to);
        state.selection.anchor = state.buffer.clamp(selection.anchor);
        state.selection.head = state.buffer.clamp(selection.head);
    }

    /// Copies the changed block at `row` from `from` to the other side.
    pub(super) fn copy_block(&mut self, row: usize, from: DiffSide, cx: &mut Context<Self>) {
        if let Some(rows) = self.change_rows_around(row) {
            self.copy_rows(rows, from, cx);
        }
    }

    /// Copies the line(s) shown in `row` from `from` to the other side.
    pub(super) fn copy_line(&mut self, row: usize, from: DiffSide, cx: &mut Context<Self>) {
        if self.diff.rows().get(row).is_some_and(|r| r.kind != RowKind::Equal) {
            self.copy_rows(row..row + 1, from, cx);
        }
    }

    /// The side and row a command acts on: the row last right-clicked if
    /// the context menu is open, otherwise the cursor's.
    pub(super) fn command_target(&self) -> (DiffSide, usize) {
        self.context_row.unwrap_or((self.active, self.cursor_row()))
    }

    /// Which context-menu edits apply to the target row.
    pub(super) fn edit_menu_state(&self) -> EditMenuState {
        let (side, row) = self.command_target();
        let changed = self.diff.rows().get(row).is_some_and(|r| r.kind != RowKind::Equal);
        let writable = |side: DiffSide| self.side(side).read_only.is_none();
        EditMenuState {
            insert_line: writable(side),
            copy_left: changed && writable(DiffSide::Left),
            copy_right: changed && writable(DiffSide::Right),
        }
    }

    /// Inserts an empty line where the target row is on its side, pushing
    /// that line down (or filling a blank spacer row), and puts the cursor
    /// on it.
    pub(super) fn insert_line(&mut self, cx: &mut Context<Self>) {
        let (side, row) = self.command_target();
        self.context_row = None;
        let at = self.diff.lines_before_row(side, row);
        let Some((range, text)) =
            line_replacement(&self.side(side).buffer, at..at, &[String::new()])
        else {
            return;
        };
        self.active = side;
        if self.edit(side, range, &text, EditKind::Other, cx) {
            self.set_cursor(Position::new(at, 0));
            self.reveal_cursor();
        }
    }

    /// Copies the line(s) in the target row towards `to`.
    pub(super) fn copy_target_line(&mut self, to: DiffSide, cx: &mut Context<Self>) {
        let (_, row) = self.command_target();
        self.context_row = None;
        self.copy_line(row, to.other(), cx);
    }

    /// Copies the changed section around the target row towards `to`.
    pub(super) fn copy_target_section(&mut self, to: DiffSide, cx: &mut Context<Self>) {
        let (_, row) = self.command_target();
        self.context_row = None;
        self.copy_block(row, to.other(), cx);
    }

    /// The row holding the cursor.
    pub(super) fn cursor_row(&self) -> usize {
        let head = self.side(self.active).selection.head;
        self.diff.row_of_line(self.active, head.line)
    }
}

#[cfg(test)]
mod tests {
    use compare_core::Encoding;

    use super::*;

    fn apply(text: &str, lines: Range<usize>, new_lines: &[&str]) -> String {
        let mut buffer = EditBuffer::from_text(text, Encoding::Utf8);
        let new_lines: Vec<String> = new_lines.iter().map(|s| s.to_string()).collect();
        if let Some((range, replacement)) = line_replacement(&buffer, lines, &new_lines) {
            buffer.replace(range, &replacement);
        }
        buffer.to_text()
    }

    #[test]
    fn replaces_inserts_and_deletes_whole_lines() {
        assert_eq!(apply("a\nb\nc\n", 1..2, &["X", "Y"]), "a\nX\nY\nc\n");
        assert_eq!(apply("a\nc\n", 1..1, &["b"]), "a\nb\nc\n");
        assert_eq!(apply("a\nb\nc\n", 1..2, &[]), "a\nc\n");
        assert_eq!(apply("a\nb", 1..2, &[]), "a");
        assert_eq!(apply("a", 0..1, &[]), "");
        assert_eq!(apply("a\nb\n", 0..0, &["z"]), "z\na\nb\n");
    }
}
