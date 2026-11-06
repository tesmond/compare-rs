//! A member of an archive.

/// A file or directory stored in an archive.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArchiveEntry {
    /// Path of the entry inside the archive, using `/` separators.
    pub name: String,
    /// Uncompressed size in bytes, or 0 when the format does not record it.
    pub size: u64,
    pub is_dir: bool,
}
