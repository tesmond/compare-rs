//! Side-by-side text comparison.

use compare_diff::{ChunkKind, DiffChunk, DiffOptions};

use crate::compare_options::CompareOptions;
use crate::text_document::TextDocument;

/// How a displayed row differs between the two sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowKind {
    Equal,
    /// Both sides have a line, and they differ.
    Changed,
    /// Only the left side has a line.
    Deleted,
    /// Only the right side has a line.
    Inserted,
}

/// One row of a side-by-side view, pairing up to one line from each side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AlignedRow {
    pub left: Option<u32>,
    pub right: Option<u32>,
    pub kind: RowKind,
}

/// A text comparison laid out as aligned rows.
#[derive(Clone, Debug)]
pub struct TextDiff {
    pub left: TextDocument,
    pub right: TextDocument,
    pub chunks: Vec<DiffChunk>,
    /// Display rows; each changed region is padded so both sides line up.
    pub rows: Vec<AlignedRow>,
    /// Index into `rows` of the first row of every changed region, in order.
    pub change_starts: Vec<usize>,
    /// True if either side was reformatted by the beautifier, in which case
    /// line numbers refer to the reformatted text.
    pub beautified: bool,
}

impl TextDiff {
    /// Diffs the two documents and aligns them into rows.
    pub fn new(left: TextDocument, right: TextDocument, options: &DiffOptions) -> Self {
        let chunks = compare_diff::diff_lines(&left.lines(), &right.lines(), options);
        let mut rows: Vec<AlignedRow> = Vec::with_capacity(left.len().max(right.len()));
        let mut change_starts: Vec<usize> = Vec::new();
        for chunk in &chunks {
            if chunk.is_change() {
                change_starts.push(rows.len());
            }
            push_rows(&mut rows, chunk, 0, 0);
        }
        Self { left, right, chunks, rows, change_starts, beautified: false }
    }

    /// Like [`TextDiff::new`], but first applies the preprocessing that
    /// `options` asks for.
    pub fn with_options(left: TextDocument, right: TextDocument, options: &CompareOptions) -> Self {
        let (left, left_changed) = preprocess(left, options);
        let (right, right_changed) = preprocess(right, options);
        let mut diff = Self::new(left, right, &options.diff);
        diff.beautified = left_changed || right_changed;
        diff
    }

    /// Returns true if any lines differ (under the diff options used).
    pub fn has_changes(&self) -> bool {
        !self.change_starts.is_empty()
    }

    /// Number of changed regions.
    pub fn change_count(&self) -> usize {
        self.change_starts.len()
    }
}

/// Beautifies `document` if requested; returns whether it changed.
fn preprocess(document: TextDocument, options: &CompareOptions) -> (TextDocument, bool) {
    if !options.beautify {
        return (document, false);
    }
    match compare_beautify::beautify(document.text()) {
        std::borrow::Cow::Owned(text) => (TextDocument::new(text), true),
        std::borrow::Cow::Borrowed(_) => (document, false),
    }
}

/// Appends the rows for `chunk`, whose line ranges are relative to
/// `left_base` and `right_base`.
pub(crate) fn push_rows(
    rows: &mut Vec<AlignedRow>,
    chunk: &DiffChunk,
    left_base: usize,
    right_base: usize,
) {
    let left_len = chunk.left.len();
    let right_len = chunk.right.len();
    for offset in 0..left_len.max(right_len) {
        let left = (offset < left_len).then(|| (left_base + chunk.left.start + offset) as u32);
        let right = (offset < right_len).then(|| (right_base + chunk.right.start + offset) as u32);
        let kind = match (chunk.kind, left.is_some(), right.is_some()) {
            (ChunkKind::Equal, _, _) => RowKind::Equal,
            (_, true, true) => RowKind::Changed,
            (_, true, false) => RowKind::Deleted,
            (_, false, _) => RowKind::Inserted,
        };
        rows.push(AlignedRow { left, right, kind });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> TextDocument {
        TextDocument::new(text.to_owned())
    }

    #[test]
    fn aligns_replacements() {
        let diff = TextDiff::new(doc("a\nb\nc\n"), doc("a\nB\nX\nc\n"), &DiffOptions::default());
        let kinds: Vec<RowKind> = diff.rows.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [RowKind::Equal, RowKind::Changed, RowKind::Inserted, RowKind::Equal]);
        assert_eq!(diff.change_starts, [1]);
        assert_eq!(
            diff.rows[2],
            AlignedRow { left: None, right: Some(2), kind: RowKind::Inserted }
        );
    }

    #[test]
    fn beautify_splits_compact_json() {
        let options = CompareOptions { beautify: true, ..Default::default() };
        let diff = TextDiff::with_options(
            doc(r#"{"a": 1, "b": 2}"#),
            doc(r#"{"a": 1, "b": 3}"#),
            &options,
        );
        assert!(diff.beautified);
        assert_eq!(diff.change_count(), 1);
        assert_eq!(diff.rows.len(), 4);
    }

    #[test]
    fn different_sizes_still_diff() {
        let diff = TextDiff::new(doc("x\n"), doc("x\ny\nz\n"), &DiffOptions::default());
        assert!(diff.has_changes());
        assert_eq!(diff.rows.len(), 3);
    }
}
