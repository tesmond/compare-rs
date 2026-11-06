//! File metadata shown alongside comparisons.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Metadata about one side of a comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl FileInfo {
    /// Reads metadata for `path`; missing metadata is reported as size 0.
    pub fn from_path(path: &Path) -> Self {
        let metadata = path.metadata().ok();
        Self {
            path: path.to_owned(),
            size: metadata.as_ref().map_or(0, |m| m.len()),
            modified: metadata.and_then(|m| m.modified().ok()),
        }
    }

    /// The final path component, or the whole path if there is none.
    pub fn display_name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
}
