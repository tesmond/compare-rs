//! Errors produced by archive operations.

use std::io;

use thiserror::Error;

/// An error listing or extracting an archive.
#[derive(Error, Debug)]
pub enum ArchiveError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("not a recognized archive format")]
    UnknownFormat,
    #[error("entry not found: {0}")]
    EntryNotFound(String),
    #[error("external tool not found: {0} (install it with Homebrew to open this archive)")]
    ToolNotFound(&'static str),
    #[error("{tool} failed: {message}")]
    ToolFailed { tool: &'static str, message: String },
}
