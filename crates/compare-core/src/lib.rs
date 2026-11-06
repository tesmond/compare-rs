//! Comparison model for compare-rs.
//!
//! - [`compare_files`] produces a text or binary comparison of two files.
//! - [`compare_folders`] recursively compares two directory trees in parallel.
//! - [`MergeDocument`] holds a three-way merge and its conflict resolutions.
//! - [`CompareTarget`] classifies paths and unpacks archives for comparison.
//! - [`EditBuffer`], [`History`], and [`LiveDiff`] support editing a comparison.

mod binary_diff;
mod compare_options;
mod compare_target;
mod edit_buffer;
mod edit_history;
mod file_comparison;
mod file_info;
mod folder_comparison;
mod live_diff;
mod merge_document;
mod text_diff;
mod text_document;

pub use binary_diff::{BYTES_PER_ROW, BinaryDiff};
pub use compare_options::CompareOptions;
pub use compare_target::{CompareTarget, TargetKind};
pub use edit_buffer::{EditBuffer, LineChange, LineEnding, Position};
pub use edit_history::{Edit, EditKind, History, Transaction};
pub use file_comparison::{ComparisonContent, FileComparison, compare_files};
pub use file_info::FileInfo;
pub use folder_comparison::{
    EntryKind, EntryMeta, FolderComparison, FolderNode, FolderProgress, FolderStats, FolderStatus,
    compare_folders,
};
pub use live_diff::{DiffSide, LiveDiff, RowSplice};
pub use merge_document::{MergeDocument, MergeRow, OutputLine, Resolution};
pub use text_diff::{AlignedRow, RowKind, TextDiff};
pub use text_document::{Encoding, TextDocument};

pub use compare_diff::DiffOptions;
