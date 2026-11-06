//! A contiguous region of a line diff.

use std::ops::Range;

/// How a [`DiffChunk`] relates the left and right sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChunkKind {
    /// The lines are the same on both sides.
    Equal,
    /// The lines exist only on the left.
    Delete,
    /// The lines exist only on the right.
    Insert,
    /// Left lines were replaced by (a possibly different number of) right lines.
    Replace,
}

/// A run of lines with a single [`ChunkKind`].
///
/// `left` and `right` are line-index ranges; one of them is empty for
/// [`ChunkKind::Delete`] and [`ChunkKind::Insert`] chunks, and they have equal
/// length for [`ChunkKind::Equal`] chunks.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiffChunk {
    pub kind: ChunkKind,
    pub left: Range<usize>,
    pub right: Range<usize>,
}

impl DiffChunk {
    /// Returns true if the chunk represents a difference.
    pub fn is_change(&self) -> bool {
        self.kind != ChunkKind::Equal
    }
}
