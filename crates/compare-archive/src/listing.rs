//! Listing archive members.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::archive_entry::ArchiveEntry;
use crate::archive_error::ArchiveError;
use crate::archive_kind::{ArchiveKind, Compression};
use crate::external_tool;

/// Lists the members of the archive at `path`.
pub fn list_entries(path: &Path) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    match ArchiveKind::from_path(path).ok_or(ArchiveError::UnknownFormat)? {
        ArchiveKind::Zip => list_zip(path),
        ArchiveKind::Tar(compression) => list_tar(path, compression),
        ArchiveKind::SingleFile(_) => Ok(vec![ArchiveEntry {
            name: ArchiveKind::single_file_name(path),
            size: 0,
            is_dir: false,
        }]),
        ArchiveKind::Rar => external_tool::list_rar(path),
        ArchiveKind::SevenZip => external_tool::list_7z(path),
    }
}

fn list_zip(path: &Path) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    let mut archive = zip::ZipArchive::new(BufReader::new(File::open(path)?))?;
    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index)?;
        // Skip entries with unsafe paths (absolute or containing `..`).
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        entries.push(ArchiveEntry {
            name: name.to_string_lossy().trim_end_matches('/').to_owned(),
            size: entry.size(),
            is_dir: entry.is_dir(),
        });
    }
    Ok(entries)
}

fn list_tar(path: &Path, compression: Compression) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    let mut archive = tar::Archive::new(compression.open(path)?);
    let mut entries = Vec::new();
    for entry in archive.entries()? {
        let entry = entry?;
        let header = entry.header();
        entries.push(ArchiveEntry {
            name: entry.path()?.to_string_lossy().trim_end_matches('/').to_owned(),
            size: header.size().unwrap_or(0),
            is_dir: header.entry_type().is_dir(),
        });
    }
    Ok(entries)
}
