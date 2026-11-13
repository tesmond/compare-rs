//! Comparing the files of two git branches without checking either out.
//!
//! Git already knows the content hash of every file on a branch, so two
//! branches are compared by listing their trees and matching hashes: no
//! file is read. A file's contents are only fetched when it is opened.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::folder_comparison::{
    EntryKind, EntryMeta, FolderComparison, FolderStatus, PendingNode, comparison_from_pending,
};
use crate::git_history::{GitError, git};

/// The branches of a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitBranches {
    /// The repository's top folder.
    pub root: PathBuf,
    /// Local branches, then remote-tracking ones (such as `origin/main`).
    pub names: Vec<String>,
    /// The checked-out branch, if `HEAD` is on one.
    pub current: Option<String>,
}

/// Lists the branches of the repository holding `path` (a folder or file).
pub fn list_branches(path: &Path) -> Result<GitBranches, GitError> {
    let path = fs::canonicalize(path)?;
    let folder = if path.is_dir() { path.as_path() } else { path.parent().unwrap_or(&path) };
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).trim().to_owned();
    let root = git(folder, &["rev-parse", "--show-toplevel"])
        .map(text)
        .map(PathBuf::from)
        .map_err(|error| match error {
            GitError::Command(_) => GitError::NotInRepository(path.clone()),
            other => other,
        })?;
    let refs = git(&root, &["for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes"])?;
    let names = String::from_utf8_lossy(&refs)
        .lines()
        .filter_map(|name| {
            name.strip_prefix("refs/heads/").or_else(|| name.strip_prefix("refs/remotes/"))
        })
        // `origin/HEAD` only points at another remote branch.
        .filter(|name| !name.ends_with("/HEAD"))
        .map(str::to_owned)
        .collect();
    let current = git(&root, &["symbolic-ref", "-q", "--short", "HEAD"])
        .ok()
        .map(text)
        .filter(|name| !name.is_empty());
    Ok(GitBranches { root, names, current })
}

/// Compares the committed files of `left` and `right` (branch names, or
/// anything else git resolves to a commit) in the repository at `root`.
///
/// The result has the shape of a folder comparison, so the same view shows
/// it. Its roots are `left_dir` and `right_dir`, which start out empty:
/// [`export_branch_file`] writes a file there when it is opened. Sizes come
/// from git; modification times are left out, since git does not record
/// them.
pub fn compare_branches(
    root: &Path,
    left: &str,
    right: &str,
    left_dir: &Path,
    right_dir: &Path,
) -> Result<FolderComparison, GitError> {
    let left_tree = read_tree(root, left)?;
    let right_tree = read_tree(root, right)?;
    let pending = merge_trees(Some(&left_tree), Some(&right_tree), Path::new(""));
    Ok(comparison_from_pending(left_dir.to_owned(), right_dir.to_owned(), pending))
}

/// Writes the file at `relative` on `branch` to `dir/relative`, unless it
/// is already there, and returns that path.
pub fn export_branch_file(
    root: &Path,
    branch: &str,
    relative: &Path,
    dir: &Path,
) -> Result<PathBuf, GitError> {
    let target = dir.join(relative);
    if target.exists() {
        return Ok(target);
    }
    // Git names paths with forward slashes on every platform.
    let path = relative.iter().map(|part| part.to_string_lossy()).collect::<Vec<_>>().join("/");
    let contents = git(root, &["cat-file", "blob", &format!("{branch}:{path}")])?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&target, contents)?;
    Ok(target)
}

/// One folder of a commit's tree.
#[derive(Default)]
struct Tree {
    folders: BTreeMap<String, Tree>,
    files: BTreeMap<String, TreeFile>,
}

struct TreeFile {
    /// Hash of the contents: equal hashes mean equal files.
    object: String,
    meta: EntryMeta,
}

/// A name in a tree: a folder or a file.
enum Entry<'a> {
    Folder(&'a Tree),
    File(&'a TreeFile),
}

impl Tree {
    fn get(&self, name: &str) -> Option<Entry<'_>> {
        self.folders.get(name).map(Entry::Folder).or_else(|| self.files.get(name).map(Entry::File))
    }

    fn names(&self) -> impl Iterator<Item = &String> {
        self.folders.keys().chain(self.files.keys())
    }
}

/// Lists every file of `commit` with its content hash and size.
fn read_tree(root: &Path, commit: &str) -> Result<Tree, GitError> {
    // The marker keeps a branch name from being read as an option.
    let output = git(root, &["ls-tree", "-r", "-l", "-z", "--end-of-options", commit])?;
    let mut tree = Tree::default();
    // Each record is "<mode> <type> <object> <size>\t<path>".
    for record in output.split(|&byte| byte == 0) {
        let record = String::from_utf8_lossy(record);
        let Some((info, path)) = record.split_once('\t') else {
            continue;
        };
        let mut fields = info.split_whitespace();
        let (Some(mode), Some(kind), Some(object), Some(size)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let kind = match (mode, kind) {
            ("120000", _) => EntryKind::Symlink,
            (_, "blob") => EntryKind::File,
            // A submodule: a commit of another repository.
            _ => EntryKind::Other,
        };
        let meta = EntryMeta { kind, size: size.parse().unwrap_or(0), modified: None };
        let mut parts = path.split('/');
        let Some(name) = parts.next_back() else {
            continue;
        };
        let mut folder = &mut tree;
        for part in parts {
            folder = folder.folders.entry(part.to_owned()).or_default();
        }
        folder.files.insert(name.to_owned(), TreeFile { object: object.to_owned(), meta });
    }
    Ok(tree)
}

/// Pairs up the names in two folders (either may be absent) and compares
/// each pair, recursing into subfolders.
fn merge_trees(left: Option<&Tree>, right: Option<&Tree>, relative_dir: &Path) -> Vec<PendingNode> {
    let folder_meta = EntryMeta { kind: EntryKind::Dir, size: 0, modified: None };
    let meta_of = |entry: &Option<Entry<'_>>| match entry {
        Some(Entry::Folder(_)) => Some(folder_meta),
        Some(Entry::File(file)) => Some(file.meta),
        None => None,
    };
    fn subfolder<'a>(entry: &Option<Entry<'a>>) -> Option<&'a Tree> {
        match entry {
            Some(Entry::Folder(tree)) => Some(tree),
            _ => None,
        }
    }
    let mut names: Vec<&String> = left.into_iter().chain(right).flat_map(Tree::names).collect();
    names.sort_unstable();
    names.dedup();

    names
        .into_iter()
        .map(|name| {
            let left_entry = left.and_then(|tree| tree.get(name));
            let right_entry = right.and_then(|tree| tree.get(name));
            let relative_path = relative_dir.join(name);
            let (left_folder, right_folder) = (subfolder(&left_entry), subfolder(&right_entry));
            let children = if left_folder.is_some() || right_folder.is_some() {
                merge_trees(left_folder, right_folder, &relative_path)
            } else {
                Vec::new()
            };
            let status = match (&left_entry, &right_entry) {
                (Some(_), None) => FolderStatus::LeftOnly,
                (None, Some(_)) => FolderStatus::RightOnly,
                (None, None) => FolderStatus::Error,
                (Some(Entry::Folder(_)), Some(Entry::Folder(_))) => {
                    if children.iter().all(|child| child.status == FolderStatus::Identical) {
                        FolderStatus::Identical
                    } else {
                        FolderStatus::Different
                    }
                }
                (Some(Entry::File(l)), Some(Entry::File(r)))
                    if l.object == r.object && l.meta.kind == r.meta.kind =>
                {
                    FolderStatus::Identical
                }
                _ => FolderStatus::Different,
            };
            PendingNode {
                name: OsString::from(name),
                relative_path,
                left: meta_of(&left_entry),
                right: meta_of(&right_entry),
                status,
                children,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .expect("run git")
            .status;
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("create folder");
        fs::write(path, text).expect("write");
    }

    /// A repository with `main` and a `feature` branch that changes one
    /// file, adds one, and removes one.
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        run(root, &["init", "-q", "-b", "main"]);
        write(root, "same.txt", "same\n");
        write(root, "src/changed.txt", "old\n");
        write(root, "src/removed.txt", "gone\n");
        run(root, &["add", "."]);
        run(root, &["commit", "-qm", "base"]);
        run(root, &["checkout", "-qb", "feature"]);
        write(root, "src/changed.txt", "new text\n");
        write(root, "docs/added.txt", "added\n");
        fs::remove_file(root.join("src/removed.txt")).expect("remove");
        run(root, &["add", "-A"]);
        run(root, &["commit", "-qm", "feature work"]);
        dir
    }

    #[test]
    fn lists_branches_and_the_current_one() {
        let dir = repo();
        let branches = list_branches(&dir.path().join("src")).expect("list");
        assert_eq!(branches.names, ["feature", "main"]);
        assert_eq!(branches.current.as_deref(), Some("feature"));
        assert_eq!(branches.root, fs::canonicalize(dir.path()).expect("canonical"));
    }

    #[test]
    fn folders_outside_a_repository_are_reported() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(matches!(list_branches(dir.path()), Err(GitError::NotInRepository(_))));
    }

    #[test]
    fn compares_two_branches_by_content() {
        let dir = repo();
        let comparison = compare_branches(
            dir.path(),
            "main",
            "feature",
            Path::new("/left"),
            Path::new("/right"),
        )
        .expect("compare");
        let status = |path: &str| {
            comparison
                .nodes
                .iter()
                .find(|node| node.relative_path == Path::new(path))
                .unwrap_or_else(|| panic!("no node for {path}"))
                .status
        };
        assert_eq!(status("same.txt"), FolderStatus::Identical);
        assert_eq!(status("src"), FolderStatus::Different);
        assert_eq!(status("src/changed.txt"), FolderStatus::Different);
        assert_eq!(status("src/removed.txt"), FolderStatus::LeftOnly);
        assert_eq!(status("docs"), FolderStatus::RightOnly);
        assert_eq!(status("docs/added.txt"), FolderStatus::RightOnly);
        let stats = comparison.stats;
        assert_eq!(
            (stats.identical, stats.different, stats.left_only, stats.right_only),
            (1, 1, 1, 1)
        );
        let changed =
            comparison.nodes.iter().find(|node| node.name == "changed.txt").expect("changed.txt");
        assert_eq!(changed.left.map(|meta| meta.size), Some(4));
        assert_eq!(changed.right.map(|meta| meta.size), Some(9));
        assert!(changed.is_file_pair());
    }

    #[test]
    fn exports_a_file_as_it_is_on_a_branch() {
        let dir = repo();
        let out = tempfile::tempdir().expect("temp dir");
        let relative = Path::new("src/changed.txt");
        let path = export_branch_file(dir.path(), "main", relative, out.path()).expect("export");
        assert_eq!(path, out.path().join(relative));
        assert_eq!(fs::read_to_string(path).expect("read"), "old\n");
    }

    #[test]
    fn unknown_branches_are_an_error() {
        let dir = repo();
        let result = compare_branches(dir.path(), "main", "nope", Path::new("/l"), Path::new("/r"));
        assert!(matches!(result, Err(GitError::Command(_))));
    }
}
