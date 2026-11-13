//! Resolving user-selected paths into something that can be compared.

use std::path::{Path, PathBuf};

use compare_archive::{ArchiveError, ArchiveKind};
use tempfile::TempDir;

/// What kind of item a path refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TargetKind {
    File,
    Folder,
    /// A file recognized as an archive by its name; compared as a folder.
    Archive,
    Missing,
}

impl TargetKind {
    /// Classifies `path` without reading file contents.
    pub fn of(path: &Path) -> Self {
        match path.metadata() {
            Ok(meta) if meta.is_dir() => TargetKind::Folder,
            Ok(_) if ArchiveKind::from_file_name(path).is_some() => TargetKind::Archive,
            Ok(_) => TargetKind::File,
            Err(_) => TargetKind::Missing,
        }
    }

    /// Returns true for targets that are compared as directory trees.
    pub fn is_folder_like(self) -> bool {
        matches!(self, TargetKind::Folder | TargetKind::Archive)
    }
}

/// A folder-like comparison target. Archives are unpacked into a temporary
/// directory that is deleted when the target is dropped.
#[derive(Debug)]
pub struct CompareTarget {
    original: PathBuf,
    extracted: Option<TempDir>,
    /// Why files in the temporary directory cannot be edited.
    read_only_reason: &'static str,
}

/// Shown on files opened from an unpacked archive.
const ARCHIVE_READ_ONLY: &str = "Read-only: inside an archive";

impl CompareTarget {
    /// Prepares `path` for folder comparison, extracting it if it is an archive.
    pub fn open_folder(path: &Path) -> Result<Self, ArchiveError> {
        let extracted = if TargetKind::of(path) == TargetKind::Archive {
            let dir = tempfile::Builder::new().prefix("compare-rs-archive-").tempdir()?;
            compare_archive::extract_all(path, dir.path())?;
            Some(dir)
        } else {
            None
        };
        Ok(Self { original: path.to_owned(), extracted, read_only_reason: ARCHIVE_READ_ONLY })
    }

    /// A temporary folder standing in for `original`, which is not a folder
    /// on disk (for example a git branch). Files are copied into it as they
    /// are opened, and are read-only for `read_only_reason`. `name` starts
    /// the folder's name, so paths shown to the user are recognizable.
    pub fn snapshot(
        original: PathBuf,
        name: &str,
        read_only_reason: &'static str,
    ) -> std::io::Result<Self> {
        let safe: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
            .collect();
        let dir = tempfile::Builder::new().prefix(&format!("{safe}-")).tempdir()?;
        Ok(Self { original, extracted: Some(dir), read_only_reason })
    }

    /// Why a file under [`CompareTarget::folder`] cannot be edited, if this
    /// target is a temporary copy.
    pub fn read_only_reason(&self) -> Option<&'static str> {
        self.extracted.as_ref().map(|_| self.read_only_reason)
    }

    /// Returns true if this is an archive unpacked into a temporary folder.
    pub fn is_extracted(&self) -> bool {
        self.extracted.is_some()
    }

    /// The path the user selected.
    pub fn original(&self) -> &Path {
        &self.original
    }

    /// The directory to compare: the extraction directory for archives,
    /// otherwise the original path.
    pub fn folder(&self) -> &Path {
        self.extracted.as_ref().map_or(&self.original, |dir| dir.path())
    }
}
