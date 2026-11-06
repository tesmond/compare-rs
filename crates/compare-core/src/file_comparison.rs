//! Comparison of two individual files.

use std::fs;
use std::io;
use std::path::Path;

use crate::binary_diff::BinaryDiff;
use crate::compare_options::CompareOptions;
use crate::file_info::FileInfo;
use crate::text_diff::TextDiff;
use crate::text_document::TextDocument;

/// How many leading bytes are scanned for NUL when sniffing for binary data.
const BINARY_SNIFF_LEN: usize = 8000;

/// The detailed comparison of two files.
#[derive(Clone, Debug)]
pub enum ComparisonContent {
    Text(TextDiff),
    Binary(BinaryDiff),
}

/// The result of comparing two files.
#[derive(Clone, Debug)]
pub struct FileComparison {
    pub left: FileInfo,
    pub right: FileInfo,
    /// True if the files are byte-for-byte identical.
    pub identical: bool,
    pub content: ComparisonContent,
}

/// Reads and compares two files.
///
/// Files are treated as binary when either contains a NUL byte near its start
/// (unless it is UTF-16 text); otherwise they are decoded and diffed by line.
pub fn compare_files(
    left: &Path,
    right: &Path,
    options: &CompareOptions,
) -> io::Result<FileComparison> {
    let (left_bytes, right_bytes) = rayon::join(|| fs::read(left), || fs::read(right));
    let (left_bytes, right_bytes) = (left_bytes?, right_bytes?);
    let identical = left_bytes == right_bytes;

    let content = if is_binary(&left_bytes) || is_binary(&right_bytes) {
        ComparisonContent::Binary(BinaryDiff::new(left_bytes, right_bytes))
    } else {
        let (left_doc, right_doc) = rayon::join(
            || TextDocument::decode(&left_bytes),
            || TextDocument::decode(&right_bytes),
        );
        ComparisonContent::Text(TextDiff::with_options(left_doc, right_doc, options))
    };

    Ok(FileComparison {
        left: FileInfo::from_path(left),
        right: FileInfo::from_path(right),
        identical,
        content,
    })
}

fn is_binary(bytes: &[u8]) -> bool {
    !TextDocument::has_utf16_bom(bytes)
        && memchr::memchr(0, &bytes[..bytes.len().min(BINARY_SNIFF_LEN)]).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).expect("write test file");
        path
    }

    #[test]
    fn text_files_of_different_size_are_diffed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let left = write(dir.path(), "l.txt", b"a\nb\n");
        let right = write(dir.path(), "r.txt", b"a\nb\nc\n");
        let result = compare_files(&left, &right, &CompareOptions::default()).expect("compare");
        assert!(!result.identical);
        let ComparisonContent::Text(diff) = result.content else {
            panic!("expected a text comparison");
        };
        assert!(diff.has_changes());
    }

    #[test]
    fn binary_files_are_detected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let left = write(dir.path(), "l.bin", b"\x00\x01\x02");
        let right = write(dir.path(), "r.bin", b"\x00\x01\x03");
        let result = compare_files(&left, &right, &CompareOptions::default()).expect("compare");
        let ComparisonContent::Binary(diff) = result.content else {
            panic!("expected a binary comparison");
        };
        assert_eq!(diff.differing_bytes, 1);
    }

    #[test]
    fn identical_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let left = write(dir.path(), "l.txt", b"same\n");
        let right = write(dir.path(), "r.txt", b"same\n");
        let result = compare_files(&left, &right, &CompareOptions::default()).expect("compare");
        assert!(result.identical);
    }
}
