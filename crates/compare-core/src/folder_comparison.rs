//! Recursive comparison of two directory trees.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::time::SystemTime;

use rayon::iter::{IntoParallelIterator, ParallelIterator};

/// File names that are never compared.
const IGNORED_NAMES: &[&str] = &[".DS_Store"];

/// Buffer size for streaming content comparison.
const COMPARE_BUFFER_LEN: usize = 128 * 1024;

/// Worker threads for folder comparison. On macOS, file-system calls from
/// many threads contend on kernel locks: comparing 20k files measured 2.4 s
/// with 14 threads but 0.31–0.38 s with 3–4.
const IO_THREADS: usize = 4;

/// The thread pool used for folder comparison, or `None` to fall back to
/// rayon's global pool if it could not be created.
fn io_pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(IO_THREADS)
            .thread_name(|index| format!("folder-compare-{index}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// The comparison status of a folder entry.
///
/// For directories present on both sides the status summarizes the contents:
/// `Identical` when every descendant is identical, `Different` otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FolderStatus {
    Identical,
    Different,
    LeftOnly,
    RightOnly,
    /// The entry could not be read on at least one side.
    Error,
}

/// The type of a file-system entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// Metadata for an entry on one side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryMeta {
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// A name present on the left, the right, or both.
#[derive(Clone, Debug)]
pub struct FolderNode {
    pub name: String,
    /// Path relative to the compared roots.
    pub relative_path: PathBuf,
    pub left: Option<EntryMeta>,
    pub right: Option<EntryMeta>,
    pub status: FolderStatus,
    pub depth: u16,
    /// Indices into [`FolderComparison::nodes`].
    pub children: Vec<usize>,
}

impl FolderNode {
    /// Returns true if the entry is a directory on either side.
    pub fn is_dir(&self) -> bool {
        [self.left, self.right].iter().flatten().any(|meta| meta.kind == EntryKind::Dir)
    }

    /// Returns true if the entry is a regular file (or symlink) on both sides.
    pub fn is_file_pair(&self) -> bool {
        let is_file = |meta: Option<EntryMeta>| {
            meta.is_some_and(|m| matches!(m.kind, EntryKind::File | EntryKind::Symlink))
        };
        is_file(self.left) && is_file(self.right)
    }
}

/// Counts of file outcomes in a folder comparison.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FolderStats {
    pub identical: usize,
    pub different: usize,
    pub left_only: usize,
    pub right_only: usize,
    pub errors: usize,
}

/// Progress shared between a running folder comparison and the UI.
#[derive(Debug, Default)]
pub struct FolderProgress {
    entries_scanned: AtomicU64,
    bytes_compared: AtomicU64,
    cancelled: AtomicBool,
}

impl FolderProgress {
    pub fn entries_scanned(&self) -> u64 {
        self.entries_scanned.load(AtomicOrdering::Relaxed)
    }

    pub fn bytes_compared(&self) -> u64 {
        self.bytes_compared.load(AtomicOrdering::Relaxed)
    }

    /// Asks the running comparison to stop as soon as possible.
    pub fn cancel(&self) {
        self.cancelled.store(true, AtomicOrdering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(AtomicOrdering::Relaxed)
    }
}

/// A completed folder comparison, stored as a flat arena of nodes.
#[derive(Clone, Debug)]
pub struct FolderComparison {
    pub left_root: PathBuf,
    pub right_root: PathBuf,
    /// All nodes in depth-first order.
    pub nodes: Vec<FolderNode>,
    /// Indices of the top-level nodes.
    pub roots: Vec<usize>,
    pub stats: FolderStats,
}

impl FolderComparison {
    pub fn left_path(&self, node: &FolderNode) -> PathBuf {
        self.left_root.join(&node.relative_path)
    }

    pub fn right_path(&self, node: &FolderNode) -> PathBuf {
        self.right_root.join(&node.relative_path)
    }
}

/// Compares the directories `left` and `right` recursively.
///
/// Subdirectories and file contents are compared in parallel. Files with
/// different sizes are reported as different without being read; files with
/// equal sizes are compared byte-for-byte, stopping at the first difference.
///
/// Returns `ErrorKind::Interrupted` if `progress` is cancelled.
pub fn compare_folders(
    left: &Path,
    right: &Path,
    progress: &FolderProgress,
) -> io::Result<FolderComparison> {
    match io_pool() {
        Some(pool) => pool.install(|| compare_folders_in_pool(left, right, progress)),
        None => compare_folders_in_pool(left, right, progress),
    }
}

fn compare_folders_in_pool(
    left: &Path,
    right: &Path,
    progress: &FolderProgress,
) -> io::Result<FolderComparison> {
    let left_entries = read_dir_sorted(left)?;
    let right_entries = read_dir_sorted(right)?;
    let pending = compare_entries(
        Some(left),
        Some(right),
        left_entries,
        right_entries,
        Path::new(""),
        progress,
    );
    if progress.is_cancelled() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "folder comparison cancelled"));
    }

    let mut comparison = FolderComparison {
        left_root: left.to_owned(),
        right_root: right.to_owned(),
        nodes: Vec::new(),
        roots: Vec::new(),
        stats: FolderStats::default(),
    };
    comparison.roots = flatten(pending, 0, &mut comparison.nodes, &mut comparison.stats);
    Ok(comparison)
}

/// A node before it is flattened into the arena.
struct PendingNode {
    name: OsString,
    relative_path: PathBuf,
    left: Option<EntryMeta>,
    right: Option<EntryMeta>,
    status: FolderStatus,
    children: Vec<PendingNode>,
}

struct DirEntryInfo {
    name: OsString,
    meta: EntryMeta,
}

/// Lists a directory, sorted by name, skipping ignored names.
fn read_dir_sorted(dir: &Path) -> io::Result<Vec<DirEntryInfo>> {
    let mut entries: Vec<DirEntryInfo> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if IGNORED_NAMES.iter().any(|ignored| name == *ignored) {
            continue;
        }
        entries.push(DirEntryInfo { meta: entry_meta(&entry.path()), name });
    }
    entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn entry_meta(path: &Path) -> EntryMeta {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            let file_type = meta.file_type();
            let kind = if file_type.is_symlink() {
                EntryKind::Symlink
            } else if file_type.is_dir() {
                EntryKind::Dir
            } else if file_type.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            EntryMeta { kind, size: meta.len(), modified: meta.modified().ok() }
        }
        Err(_) => EntryMeta { kind: EntryKind::Other, size: 0, modified: None },
    }
}

/// Pairs up two sorted directory listings by name and compares each pair.
fn compare_entries(
    left_dir: Option<&Path>,
    right_dir: Option<&Path>,
    left_entries: Vec<DirEntryInfo>,
    right_entries: Vec<DirEntryInfo>,
    relative_dir: &Path,
    progress: &FolderProgress,
) -> Vec<PendingNode> {
    let mut pairs: Vec<(OsString, Option<EntryMeta>, Option<EntryMeta>)> =
        Vec::with_capacity(left_entries.len().max(right_entries.len()));
    let mut left_iter = left_entries.into_iter().peekable();
    let mut right_iter = right_entries.into_iter().peekable();
    loop {
        let order = match (left_iter.peek(), right_iter.peek()) {
            (Some(l), Some(r)) => l.name.cmp(&r.name),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => break,
        };
        let pair = match order {
            Ordering::Less => left_iter.next().map(|l| (l.name, Some(l.meta), None)),
            Ordering::Greater => right_iter.next().map(|r| (r.name, None, Some(r.meta))),
            Ordering::Equal => left_iter
                .next()
                .zip(right_iter.next())
                .map(|(l, r)| (l.name, Some(l.meta), Some(r.meta))),
        };
        pairs.extend(pair);
    }

    pairs
        .into_par_iter()
        .map(|(name, left, right)| {
            let relative_path = relative_dir.join(&name);
            let left_path = left_dir.map(|dir| dir.join(&name));
            let right_path = right_dir.map(|dir| dir.join(&name));
            compare_pair(name, relative_path, left, right, left_path, right_path, progress)
        })
        .collect()
}

fn compare_pair(
    name: OsString,
    relative_path: PathBuf,
    left: Option<EntryMeta>,
    right: Option<EntryMeta>,
    left_path: Option<PathBuf>,
    right_path: Option<PathBuf>,
    progress: &FolderProgress,
) -> PendingNode {
    progress.entries_scanned.fetch_add(1, AtomicOrdering::Relaxed);
    let is_dir = |meta: Option<EntryMeta>| meta.is_some_and(|m| m.kind == EntryKind::Dir);
    let left_dir = left_path.as_deref().filter(|_| is_dir(left));
    let right_dir = right_path.as_deref().filter(|_| is_dir(right));

    let mut children = Vec::new();
    let mut read_error = false;
    if (left_dir.is_some() || right_dir.is_some()) && !progress.is_cancelled() {
        let mut list = |dir: Option<&Path>| -> Vec<DirEntryInfo> {
            dir.map_or_else(Vec::new, |dir| {
                read_dir_sorted(dir).unwrap_or_else(|_| {
                    read_error = true;
                    Vec::new()
                })
            })
        };
        let left_entries = list(left_dir);
        let right_entries = list(right_dir);
        children = compare_entries(
            left_dir,
            right_dir,
            left_entries,
            right_entries,
            &relative_path,
            progress,
        );
    }

    let status = match (left, right) {
        (Some(_), None) => FolderStatus::LeftOnly,
        (None, Some(_)) => FolderStatus::RightOnly,
        (None, None) => FolderStatus::Error,
        (Some(l), Some(r)) => {
            if read_error {
                FolderStatus::Error
            } else if l.kind == EntryKind::Dir && r.kind == EntryKind::Dir {
                let all_identical = children.iter().all(|c| c.status == FolderStatus::Identical);
                if all_identical { FolderStatus::Identical } else { FolderStatus::Different }
            } else if l.kind != r.kind {
                FolderStatus::Different
            } else {
                match (&left_path, &right_path) {
                    (Some(lp), Some(rp)) => compare_file_contents(lp, rp, l, r, progress),
                    _ => FolderStatus::Error,
                }
            }
        }
    };

    PendingNode { name, relative_path, left, right, status, children }
}

fn compare_file_contents(
    left_path: &Path,
    right_path: &Path,
    left: EntryMeta,
    right: EntryMeta,
    progress: &FolderProgress,
) -> FolderStatus {
    let result = if left.kind == EntryKind::Symlink {
        fs::read_link(left_path).and_then(|l| fs::read_link(right_path).map(|r| l == r))
    } else if left.size != right.size {
        Ok(false)
    } else {
        streams_equal(left_path, right_path, progress)
    };
    match result {
        Ok(true) => FolderStatus::Identical,
        Ok(false) => FolderStatus::Different,
        Err(_) => FolderStatus::Error,
    }
}

/// Compares two files chunk by chunk, stopping at the first difference.
fn streams_equal(left: &Path, right: &Path, progress: &FolderProgress) -> io::Result<bool> {
    thread_local! {
        // Reused across files so comparing many small files does not
        // allocate a fresh pair of buffers each time.
        static BUFFERS: RefCell<(Vec<u8>, Vec<u8>)> =
            RefCell::new((vec![0; COMPARE_BUFFER_LEN], vec![0; COMPARE_BUFFER_LEN]));
    }
    let mut left_file = File::open(left)?;
    let mut right_file = File::open(right)?;
    BUFFERS.with_borrow_mut(|(left_buf, right_buf)| {
        loop {
            if progress.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            let left_len = read_full(&mut left_file, left_buf)?;
            let right_len = read_full(&mut right_file, right_buf)?;
            progress.bytes_compared.fetch_add(left_len as u64, AtomicOrdering::Relaxed);
            if left_buf[..left_len] != right_buf[..right_len] {
                return Ok(false);
            }
            // A short read means both files reached their end.
            if left_len < left_buf.len() {
                return Ok(true);
            }
        }
    })
}

fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Moves pending nodes into the arena in depth-first order, directories first,
/// returning the indices of `pending`'s nodes.
fn flatten(
    mut pending: Vec<PendingNode>,
    depth: u16,
    nodes: &mut Vec<FolderNode>,
    stats: &mut FolderStats,
) -> Vec<usize> {
    pending.sort_by(|a, b| {
        let a_dir = pending_is_dir(a);
        let b_dir = pending_is_dir(b);
        b_dir.cmp(&a_dir).then_with(|| compare_names(&a.name, &b.name))
    });
    let mut indices = Vec::with_capacity(pending.len());
    for node in pending {
        let index = nodes.len();
        indices.push(index);
        if !pending_is_dir(&node) {
            match node.status {
                FolderStatus::Identical => stats.identical += 1,
                FolderStatus::Different => stats.different += 1,
                FolderStatus::LeftOnly => stats.left_only += 1,
                FolderStatus::RightOnly => stats.right_only += 1,
                FolderStatus::Error => stats.errors += 1,
            }
        }
        nodes.push(FolderNode {
            name: node.name.to_string_lossy().into_owned(),
            relative_path: node.relative_path,
            left: node.left,
            right: node.right,
            status: node.status,
            depth,
            children: Vec::new(),
        });
        let children = flatten(node.children, depth + 1, nodes, stats);
        nodes[index].children = children;
    }
    indices
}

fn pending_is_dir(node: &PendingNode) -> bool {
    [node.left, node.right].iter().flatten().any(|meta| meta.kind == EntryKind::Dir)
}

/// Case-insensitive name order with a case-sensitive tiebreak.
fn compare_names(a: &OsString, b: &OsString) -> Ordering {
    let a = a.to_string_lossy();
    let b = b.to_string_lossy();
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, contents).expect("write");
    }

    fn status_of(comparison: &FolderComparison, relative: &str) -> FolderStatus {
        comparison
            .nodes
            .iter()
            .find(|n| n.relative_path == Path::new(relative))
            .map(|n| n.status)
            .unwrap_or_else(|| panic!("no node for {relative}"))
    }

    #[test]
    fn annotates_all_statuses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let left = dir.path().join("left");
        let right = dir.path().join("right");
        write(&left.join("same.txt"), "same");
        write(&right.join("same.txt"), "same");
        write(&left.join("diff.txt"), "aaaa");
        write(&right.join("diff.txt"), "bbbb");
        write(&left.join("left_only.txt"), "l");
        write(&right.join("right_only.txt"), "r");
        write(&left.join("sub/deep.txt"), "x");
        write(&right.join("sub/deep.txt"), "x");
        write(&left.join("changed/inner.txt"), "1");
        write(&right.join("changed/inner.txt"), "22");
        write(&right.join("new_dir/file.txt"), "n");

        let comparison =
            compare_folders(&left, &right, &FolderProgress::default()).expect("compare");
        assert_eq!(status_of(&comparison, "same.txt"), FolderStatus::Identical);
        assert_eq!(status_of(&comparison, "diff.txt"), FolderStatus::Different);
        assert_eq!(status_of(&comparison, "left_only.txt"), FolderStatus::LeftOnly);
        assert_eq!(status_of(&comparison, "right_only.txt"), FolderStatus::RightOnly);
        assert_eq!(status_of(&comparison, "sub"), FolderStatus::Identical);
        assert_eq!(status_of(&comparison, "changed"), FolderStatus::Different);
        assert_eq!(status_of(&comparison, "new_dir"), FolderStatus::RightOnly);
        assert_eq!(status_of(&comparison, "new_dir/file.txt"), FolderStatus::RightOnly);

        let stats = comparison.stats;
        assert_eq!((stats.identical, stats.different), (2, 2));
        assert_eq!((stats.left_only, stats.right_only), (1, 2));

        // Directories sort before files at each level.
        let first = &comparison.nodes[comparison.roots[0]];
        assert!(first.is_dir());
    }

    #[test]
    fn cancellation_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let progress = FolderProgress::default();
        progress.cancel();
        let result = compare_folders(dir.path(), dir.path(), &progress);
        assert_eq!(result.map(|_| ()).map_err(|e| e.kind()), Err(io::ErrorKind::Interrupted));
    }
}
