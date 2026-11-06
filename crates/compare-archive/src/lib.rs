//! Archive support for compare-rs.
//!
//! Handles ZIP, TAR (optionally gzip/bzip2/xz compressed), single-stream
//! GZ/BZ2/XZ files, and RAR/7z via the external `unar`/`lsar` and `7z` tools.

mod archive_entry;
mod archive_error;
mod archive_kind;
mod external_tool;
mod extraction;
mod listing;

pub use archive_entry::ArchiveEntry;
pub use archive_error::ArchiveError;
pub use archive_kind::{ArchiveKind, Compression};
pub use extraction::{extract_all, extract_entry};
pub use listing::list_entries;
