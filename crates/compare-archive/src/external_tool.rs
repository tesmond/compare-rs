//! RAR and 7z support through external command-line tools.
//!
//! RAR uses `unar`/`lsar` (from The Unarchiver) when installed and falls back
//! to `7z`, which also reads RAR archives.

use std::io::{ErrorKind, Write};
use std::path::Path;
use std::process::{Command, Output};

use crate::archive_entry::ArchiveEntry;
use crate::archive_error::ArchiveError;

/// Runs `tool` with `args` and returns its output, failing on a non-zero exit.
fn run(tool: &'static str, args: &[&std::ffi::OsStr]) -> Result<Output, ArchiveError> {
    let output = Command::new(tool).args(args).output().map_err(|e| match e.kind() {
        ErrorKind::NotFound => ArchiveError::ToolNotFound(tool),
        _ => ArchiveError::Io(e),
    })?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(ArchiveError::ToolFailed { tool, message });
    }
    Ok(output)
}

/// Returns `primary`'s result unless the tool is missing, in which case
/// `fallback` is tried.
fn with_fallback<T>(
    primary: impl FnOnce() -> Result<T, ArchiveError>,
    fallback: impl FnOnce() -> Result<T, ArchiveError>,
) -> Result<T, ArchiveError> {
    match primary() {
        Err(ArchiveError::ToolNotFound(_)) => fallback(),
        result => result,
    }
}

pub(crate) fn list_rar(path: &Path) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    with_fallback(
        || {
            let output = run("lsar", &[path.as_os_str()])?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            // The first line names the archive; each following line is an entry,
            // with directories ending in '/'.
            let entries = stdout
                .lines()
                .skip(1)
                .filter(|line| !line.trim().is_empty())
                .map(|line| ArchiveEntry {
                    name: line.trim_end_matches('/').to_owned(),
                    size: 0,
                    is_dir: line.ends_with('/'),
                })
                .collect();
            Ok(entries)
        },
        || list_7z(path),
    )
}

pub(crate) fn list_7z(path: &Path) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    let output = run("7z", &["l".as_ref(), "-slt".as_ref(), "-ba".as_ref(), path.as_os_str()])?;
    Ok(parse_7z_technical_listing(&String::from_utf8_lossy(&output.stdout)))
}

/// Parses `7z l -slt` output: blocks of `Key = Value` lines separated by blank
/// lines, one block per entry.
fn parse_7z_technical_listing(stdout: &str) -> Vec<ArchiveEntry> {
    let mut entries = Vec::new();
    let mut current: Option<ArchiveEntry> = None;
    for line in stdout.lines() {
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        match key {
            "Path" => {
                entries.extend(current.take());
                current = Some(ArchiveEntry { name: value.to_owned(), size: 0, is_dir: false });
            }
            "Size" => {
                if let Some(entry) = current.as_mut() {
                    entry.size = value.parse().unwrap_or(0);
                }
            }
            "Folder" => {
                if let Some(entry) = current.as_mut() {
                    entry.is_dir |= value == "+";
                }
            }
            "Attributes" => {
                if let Some(entry) = current.as_mut() {
                    entry.is_dir |= value.starts_with('D');
                }
            }
            _ => {}
        }
    }
    entries.extend(current);
    entries
}

pub(crate) fn extract_all_rar(path: &Path, dest: &Path) -> Result<(), ArchiveError> {
    with_fallback(
        || {
            let args = [
                "-q".as_ref(),
                "-f".as_ref(),
                "-D".as_ref(),
                "-o".as_ref(),
                dest.as_os_str(),
                path.as_os_str(),
            ];
            run("unar", &args).map(drop)
        },
        || extract_all_7z(path, dest),
    )
}

pub(crate) fn extract_all_7z(path: &Path, dest: &Path) -> Result<(), ArchiveError> {
    let mut out_flag = std::ffi::OsString::from("-o");
    out_flag.push(dest);
    run("7z", &["x".as_ref(), "-y".as_ref(), out_flag.as_os_str(), path.as_os_str()]).map(drop)
}

pub(crate) fn extract_entry_7z<W: Write>(
    path: &Path,
    entry_name: &str,
    output: &mut W,
) -> Result<(), ArchiveError> {
    let result = run("7z", &["e".as_ref(), "-so".as_ref(), path.as_os_str(), entry_name.as_ref()])?;
    output.write_all(&result.stdout)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_7z_listing() {
        let listing = "Path = docs\nSize = 0\nAttributes = D....\n\n\
                       Path = docs/a.txt\nSize = 12\nAttributes = ....A\n";
        let entries = parse_7z_technical_listing(listing);
        assert_eq!(
            entries,
            vec![
                ArchiveEntry { name: "docs".into(), size: 0, is_dir: true },
                ArchiveEntry { name: "docs/a.txt".into(), size: 12, is_dir: false },
            ]
        );
    }
}
