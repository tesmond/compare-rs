//! Aligned diff rows that are kept up to date while either side is edited.

use std::ops::Range;

use compare_diff::DiffOptions;

use crate::edit_buffer::LineChange;
use crate::text_diff::{AlignedRow, RowKind, TextDiff, push_rows};

/// One side of a two-way comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiffSide {
    Left,
    Right,
}

impl DiffSide {
    pub fn index(self) -> usize {
        match self {
            DiffSide::Left => 0,
            DiffSide::Right => 1,
        }
    }

    pub fn other(self) -> Self {
        match self {
            DiffSide::Left => DiffSide::Right,
            DiffSide::Right => DiffSide::Left,
        }
    }

    /// The line index a row shows on this side, if any.
    pub fn line_of(self, row: AlignedRow) -> Option<u32> {
        match self {
            DiffSide::Left => row.left,
            DiffSide::Right => row.right,
        }
    }
}

/// The rows that a [`LiveDiff::apply_edit`] call replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowSplice {
    /// Rows that were removed.
    pub old: Range<usize>,
    /// Number of rows inserted in their place.
    pub new_len: usize,
}

/// Aligned rows for two line sequences.
///
/// After an edit, only the changed region (widened to the surrounding
/// unchanged rows) is diffed again, so typing stays fast in large files.
/// That local result can align lines less well than a full diff, so callers
/// should run [`LiveDiff::compute`] again once editing pauses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveDiff {
    rows: Vec<AlignedRow>,
    /// First row of each changed region.
    change_starts: Vec<usize>,
    /// For each side, the row that shows each line.
    line_rows: [Vec<u32>; 2],
}

impl LiveDiff {
    /// Diffs two complete line sequences.
    pub fn compute<L: AsRef<str>, R: AsRef<str>>(
        left: &[L],
        right: &[R],
        options: &DiffOptions,
    ) -> Self {
        let left: Vec<&str> = left.iter().map(AsRef::as_ref).collect();
        let right: Vec<&str> = right.iter().map(AsRef::as_ref).collect();
        let mut rows = Vec::with_capacity(left.len().max(right.len()));
        for chunk in compare_diff::diff_lines(&left, &right, options) {
            push_rows(&mut rows, &chunk, 0, 0);
        }
        Self::from_rows(rows)
    }

    /// Reuses the rows of an existing comparison of the same text.
    ///
    /// A buffer has one more line than its document when the text ends with
    /// a newline (the empty line after it); `left_lines` and `right_lines`
    /// are the buffer line counts, so that line gets a row too.
    pub fn from_text_diff(diff: &TextDiff, left_lines: usize, right_lines: usize) -> Self {
        let mut rows = diff.rows.clone();
        let extra_left = (left_lines > diff.left.len()).then_some(diff.left.len() as u32);
        let extra_right = (right_lines > diff.right.len()).then_some(diff.right.len() as u32);
        let kind = match (extra_left, extra_right) {
            (Some(_), Some(_)) => Some(RowKind::Equal),
            (Some(_), None) => Some(RowKind::Deleted),
            (None, Some(_)) => Some(RowKind::Inserted),
            (None, None) => None,
        };
        if let Some(kind) = kind {
            rows.push(AlignedRow { left: extra_left, right: extra_right, kind });
        }
        Self::from_rows(rows)
    }

    fn from_rows(rows: Vec<AlignedRow>) -> Self {
        let mut diff = Self { rows, change_starts: Vec::new(), line_rows: Default::default() };
        diff.rebuild_index();
        diff
    }

    pub fn rows(&self) -> &[AlignedRow] {
        &self.rows
    }

    /// Index of the first row of every changed region, in order.
    pub fn change_starts(&self) -> &[usize] {
        &self.change_starts
    }

    pub fn has_changes(&self) -> bool {
        !self.change_starts.is_empty()
    }

    pub fn change_count(&self) -> usize {
        self.change_starts.len()
    }

    /// The row showing `line` on `side`, or the last row if out of range.
    pub fn row_of_line(&self, side: DiffSide, line: usize) -> usize {
        let rows = &self.line_rows[side.index()];
        rows.get(line).or(rows.last()).map_or(0, |&row| row as usize)
    }

    /// Number of lines of `side` shown in rows before `row`.
    pub fn lines_before_row(&self, side: DiffSide, row: usize) -> usize {
        self.line_rows[side.index()].partition_point(|&r| (r as usize) < row)
    }

    /// Updates the rows after lines of `side` changed as `change` describes.
    /// `left` and `right` are the full line sequences after the edit.
    pub fn apply_edit<L: AsRef<str>, R: AsRef<str>>(
        &mut self,
        side: DiffSide,
        change: LineChange,
        left: &[L],
        right: &[R],
        options: &DiffOptions,
    ) -> RowSplice {
        let line_rows = &self.line_rows[side.index()];
        let last_old_line = (change.first_line + change.old_count).saturating_sub(1);
        let mut start =
            line_rows.get(change.first_line).map_or(self.rows.len(), |&row| row as usize);
        let mut end = line_rows.get(last_old_line).map_or(self.rows.len(), |&row| row as usize + 1);
        // Widen to whole changed regions so they are re-aligned as a unit.
        while start > 0 && self.rows[start - 1].kind != RowKind::Equal {
            start -= 1;
        }
        while end < self.rows.len() && self.rows[end].kind != RowKind::Equal {
            end += 1;
        }

        let delta = change.new_count as isize - change.old_count as isize;
        let window = |s: DiffSide| {
            let first = self.lines_before_row(s, start);
            let last = self.lines_before_row(s, end);
            let last = if s == side { last.saturating_add_signed(delta) } else { last };
            first..last
        };
        let (left_window, right_window) = (window(DiffSide::Left), window(DiffSide::Right));
        let left_lines: Vec<&str> = left[left_window.clone()].iter().map(AsRef::as_ref).collect();
        let right_lines: Vec<&str> =
            right[right_window.clone()].iter().map(AsRef::as_ref).collect();

        let mut new_rows = Vec::with_capacity(end - start + change.new_count);
        for chunk in compare_diff::diff_lines(&left_lines, &right_lines, options) {
            push_rows(&mut new_rows, &chunk, left_window.start, right_window.start);
        }
        let new_len = new_rows.len();
        self.rows.splice(start..end, new_rows);
        if delta != 0 {
            for row in &mut self.rows[start + new_len..] {
                let line = match side {
                    DiffSide::Left => &mut row.left,
                    DiffSide::Right => &mut row.right,
                };
                if let Some(line) = line {
                    *line = line.saturating_add_signed(delta as i32);
                }
            }
        }
        self.rebuild_index();
        RowSplice { old: start..end, new_len }
    }

    fn rebuild_index(&mut self) {
        self.change_starts.clear();
        for side in &mut self.line_rows {
            side.clear();
        }
        for (index, row) in self.rows.iter().enumerate() {
            let starts_change = row.kind != RowKind::Equal
                && (index == 0 || self.rows[index - 1].kind == RowKind::Equal);
            if starts_change {
                self.change_starts.push(index);
            }
            if row.left.is_some() {
                self.line_rows[0].push(index as u32);
            }
            if row.right.is_some() {
                self.line_rows[1].push(index as u32);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit_buffer::{EditBuffer, Position};
    use crate::text_document::{Encoding, TextDocument};

    fn kinds(diff: &LiveDiff) -> Vec<RowKind> {
        diff.rows().iter().map(|r| r.kind).collect()
    }

    fn buffer(text: &str) -> EditBuffer {
        EditBuffer::from_text(text, Encoding::Utf8)
    }

    /// Applies an edit to `right` and checks the patched rows against a
    /// full diff.
    fn edit_right(left: &EditBuffer, right: &mut EditBuffer, range: Range<Position>, text: &str) {
        let options = DiffOptions::default();
        let mut diff = LiveDiff::compute(left.lines(), right.lines(), &options);
        let change = right.replace(range, text);
        diff.apply_edit(DiffSide::Right, change, left.lines(), right.lines(), &options);
        assert_eq!(diff, LiveDiff::compute(left.lines(), right.lines(), &options));
    }

    #[test]
    fn typing_a_difference_and_fixing_it() {
        let left = buffer("a\nb\nc\n");
        let mut right = buffer("a\nb\nc\n");
        edit_right(&left, &mut right, Position::new(1, 1)..Position::new(1, 1), "x");
        edit_right(&left, &mut right, Position::new(1, 1)..Position::new(1, 2), "");
    }

    #[test]
    fn inserting_and_removing_lines() {
        let left = buffer("a\nb\nc\nd\n");
        let mut right = buffer("a\nb\nc\nd\n");
        edit_right(&left, &mut right, Position::new(1, 1)..Position::new(1, 1), "\nnew\nlines");
        edit_right(&left, &mut right, Position::new(0, 1)..Position::new(3, 0), "");
    }

    #[test]
    fn edits_shift_later_rows() {
        let options = DiffOptions::default();
        let left = buffer("a\nb\nc\nd\ne\n");
        let mut right = buffer("a\nX\nc\nd\nY\n");
        let mut diff = LiveDiff::compute(left.lines(), right.lines(), &options);
        assert_eq!(diff.change_count(), 2);
        let change = right.replace(Position::new(0, 1)..Position::new(0, 1), "\n0");
        let splice =
            diff.apply_edit(DiffSide::Right, change, left.lines(), right.lines(), &options);
        // The window widens to include the changed region after the edit.
        assert_eq!(splice.old, 0..2);
        assert_eq!(diff, LiveDiff::compute(left.lines(), right.lines(), &options));
        assert_eq!(diff.change_count(), 2);
        assert_eq!(diff.row_of_line(DiffSide::Right, 5), 5);
        assert_eq!(
            kinds(&diff),
            [
                RowKind::Equal,
                RowKind::Changed,
                RowKind::Inserted,
                RowKind::Equal,
                RowKind::Equal,
                RowKind::Changed,
                RowKind::Equal,
            ]
        );
    }

    #[test]
    fn trailing_empty_line_gets_a_row() {
        let left = TextDocument::new("a\n".to_owned());
        let right = TextDocument::new("a".to_owned());
        let text_diff = TextDiff::new(left.clone(), right.clone(), &DiffOptions::default());
        let live = LiveDiff::from_text_diff(
            &text_diff,
            EditBuffer::from_document(&left).len(),
            EditBuffer::from_document(&right).len(),
        );
        assert_eq!(kinds(&live), [RowKind::Equal, RowKind::Deleted]);
        assert_eq!(live.change_starts(), [1]);
    }
}
