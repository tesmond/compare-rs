//! Fast text diff engine for compare-rs.
//!
//! - [`diff_lines`] compares two sequences of lines with a linear-space Myers
//!   algorithm, after interning lines to integers and discarding lines that
//!   cannot match.
//! - [`diff_inline`] finds the changed spans within a pair of lines.
//! - [`merge3`] performs a three-way (diff3-style) merge.

mod diff_chunk;
mod diff_options;
mod inline_diff;
mod line_diff;
mod merge3;
mod myers;

pub use diff_chunk::{ChunkKind, DiffChunk};
pub use diff_options::DiffOptions;
pub use inline_diff::{InlineDiff, diff_inline};
pub use line_diff::diff_lines;
pub use merge3::{AutoSource, MergeRegion, MergeRegionKind, merge3};
pub use myers::MatchBlock;
