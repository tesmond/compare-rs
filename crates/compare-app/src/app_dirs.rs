//! Locations of the app's files on disk.

use std::path::PathBuf;

/// `~/Library/Application Support/CompareRS`, or `None` if `HOME` is
/// not set.
pub fn support_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/CompareRS"))
}
