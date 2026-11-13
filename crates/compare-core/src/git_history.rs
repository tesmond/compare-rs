//! The versions of one file in a git repository, read with the `git`
//! command-line tool.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::file_info::FileInfo;

/// Why a file's git history could not be read.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git is not installed or could not be run: {0}")]
    GitUnavailable(io::Error),
    #[error("{0} is not inside a git repository")]
    NotInRepository(PathBuf),
    #[error("git failed: {0}")]
    Command(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A commit that changed the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub time: SystemTime,
    pub subject: String,
    /// The file's path (relative to the repository root) in this commit;
    /// it differs from the current path if the file was renamed later.
    pub path: PathBuf,
}

/// Who last changed one line, as reported by `git blame`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    pub short_hash: String,
    pub author: String,
    pub time: SystemTime,
    pub summary: String,
    /// True for lines changed in the working copy but not yet committed.
    pub uncommitted: bool,
}

/// One version of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileVersion {
    /// The file as it is on disk now.
    Working,
    Commit(Commit),
    /// Before the file existed.
    Empty,
}

/// A file in a git repository and the versions it has had.
#[derive(Clone, Debug)]
pub struct GitFile {
    root: PathBuf,
    /// The file on disk.
    path: PathBuf,
    /// Versions, newest first: the working copy, every commit that changed
    /// the file, then the empty version before it existed.
    versions: Vec<FileVersion>,
    /// True if the working copy differs from the last commit (or the file
    /// has never been committed).
    has_uncommitted_changes: bool,
}

impl GitFile {
    /// Reads the history of the file at `path`.
    pub fn open(path: &Path) -> Result<Self, GitError> {
        let Located { root, path, relative } = locate(path)?;
        let relative_arg = relative.to_string_lossy().into_owned();

        let status = git(&root, &["status", "--porcelain=v1", "--", &relative_arg])?;
        let has_uncommitted_changes = !status.is_empty();

        const RECORD: char = '\u{1e}';
        const FIELD: char = '\u{1f}';
        let log = git(
            &root,
            &[
                "log",
                "--follow",
                "--name-only",
                "--format=%x1e%H%x1f%h%x1f%an%x1f%at%x1f%s",
                "--",
                &relative_arg,
            ],
        )?;
        let log = String::from_utf8_lossy(&log);
        let mut commits = Vec::new();
        let mut last_path = relative.clone();
        for record in log.split(RECORD).filter(|r| !r.trim().is_empty()) {
            let mut lines = record.lines();
            let header = lines.next().unwrap_or_default();
            let fields: Vec<&str> = header.split(FIELD).collect();
            let [hash, short_hash, author, time, subject] = fields[..] else {
                continue;
            };
            // Merge commits list no files; they keep the last known path.
            if let Some(name) = lines.map(str::trim).find(|line| !line.is_empty()) {
                last_path = PathBuf::from(name);
            }
            let seconds = time.parse::<u64>().unwrap_or(0);
            commits.push(Commit {
                hash: hash.to_owned(),
                short_hash: short_hash.to_owned(),
                author: author.to_owned(),
                time: UNIX_EPOCH + Duration::from_secs(seconds),
                subject: subject.to_owned(),
                path: last_path.clone(),
            });
        }

        let mut versions = vec![FileVersion::Working];
        versions.extend(commits.into_iter().map(FileVersion::Commit));
        versions.push(FileVersion::Empty);
        Ok(Self { root, path, versions, has_uncommitted_changes })
    }

    /// The file on disk.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The repository's top folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// All versions, newest first (see [`GitFile`]).
    pub fn versions(&self) -> &[FileVersion] {
        &self.versions
    }

    pub fn has_uncommitted_changes(&self) -> bool {
        self.has_uncommitted_changes
    }

    /// The versions to compare first, as indices into [`GitFile::versions`]
    /// (older, newer). The newer side is always the working copy. With
    /// uncommitted changes the older side is the last commit; otherwise it
    /// is the commit before that (or the empty version if there is none).
    pub fn initial_pair(&self) -> (usize, usize) {
        let skip_identical_commit = !self.has_uncommitted_changes;
        let older = if skip_identical_commit { 2 } else { 1 };
        (older.min(self.versions.len() - 1), 0)
    }

    /// File details for version `index`, whose contents are `size` bytes:
    /// the file's own details for the working copy, the commit time for a
    /// commit, and none for the empty version.
    pub fn version_info(&self, index: usize, size: u64) -> FileInfo {
        match &self.versions[index] {
            FileVersion::Working => FileInfo::from_path(&self.path),
            FileVersion::Commit(commit) => {
                FileInfo { path: self.path.clone(), size, modified: Some(commit.time) }
            }
            FileVersion::Empty => FileInfo { path: self.path.clone(), size: 0, modified: None },
        }
    }

    /// The contents of version `index`.
    pub fn read(&self, index: usize) -> Result<Vec<u8>, GitError> {
        match &self.versions[index] {
            FileVersion::Working => Ok(fs::read(&self.path)?),
            FileVersion::Empty => Ok(Vec::new()),
            FileVersion::Commit(commit) => {
                let spec = format!("{}:{}", commit.hash, commit.path.to_string_lossy());
                git(&self.root, &["cat-file", "blob", &spec])
            }
        }
    }
}

impl GitFile {
    /// Who last changed each line of version `index` (one entry per line),
    /// or nothing for the empty version.
    pub fn blame(&self, index: usize) -> Result<Vec<BlameLine>, GitError> {
        let output = match &self.versions[index] {
            FileVersion::Empty => return Ok(Vec::new()),
            FileVersion::Working => {
                let relative = self.path.strip_prefix(&self.root).unwrap_or(&self.path);
                git(&self.root, &["blame", "--porcelain", "--", &relative.to_string_lossy()])?
            }
            FileVersion::Commit(commit) => git(
                &self.root,
                &["blame", "--porcelain", &commit.hash, "--", &commit.path.to_string_lossy()],
            )?,
        };
        Ok(parse_blame(&String::from_utf8_lossy(&output)))
    }
}

/// Parses `git blame --porcelain` output into one entry per line.
fn parse_blame(output: &str) -> Vec<BlameLine> {
    #[derive(Default, Clone)]
    struct CommitInfo {
        author: String,
        time: u64,
        summary: String,
    }
    let mut commits: std::collections::HashMap<String, CommitInfo> = Default::default();
    let mut lines: Vec<(usize, String)> = Vec::new();
    let mut current: Option<(String, usize)> = None;
    for line in output.lines() {
        if let Some(_content) = line.strip_prefix('\t') {
            if let Some((hash, final_line)) = current.take() {
                lines.push((final_line, hash));
            }
            continue;
        }
        let mut words = line.splitn(2, ' ');
        let key = words.next().unwrap_or_default();
        let value = words.next().unwrap_or_default();
        if key.len() == 40 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
            let final_line = value.split(' ').nth(1).and_then(|n| n.parse().ok()).unwrap_or(0);
            commits.entry(key.to_owned()).or_default();
            current = Some((key.to_owned(), final_line));
            continue;
        }
        let Some((hash, _)) = &current else {
            continue;
        };
        let info = commits.entry(hash.clone()).or_default();
        match key {
            "author" => info.author = value.to_owned(),
            "author-time" => info.time = value.parse().unwrap_or(0),
            "summary" => info.summary = value.to_owned(),
            _ => {}
        }
    }
    lines.sort_by_key(|(final_line, _)| *final_line);
    lines
        .into_iter()
        .map(|(_, hash)| {
            let info = commits.get(&hash).cloned().unwrap_or_default();
            let uncommitted = hash.bytes().all(|b| b == b'0');
            BlameLine {
                short_hash: hash[..7].to_owned(),
                author: info.author,
                time: UNIX_EPOCH + Duration::from_secs(info.time),
                summary: info.summary,
                uncommitted,
            }
        })
        .collect()
}

/// Runs `git` in `folder` and returns its output.
/// A file inside a repository.
struct Located {
    /// The repository's top folder.
    root: PathBuf,
    /// The file, with symlinks resolved.
    path: PathBuf,
    /// The file relative to `root`.
    relative: PathBuf,
}

fn locate(path: &Path) -> Result<Located, GitError> {
    let path = fs::canonicalize(path)?;
    let folder = path.parent().unwrap_or(Path::new("/"));
    let root = git(folder, &["rev-parse", "--show-toplevel"])
        .map_err(|error| match error {
            GitError::Command(_) => GitError::NotInRepository(path.clone()),
            other => other,
        })
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out).trim()))?;
    let root = fs::canonicalize(root)?;
    let relative = path
        .strip_prefix(&root)
        .map_err(|_| GitError::NotInRepository(path.clone()))?
        .to_path_buf();
    Ok(Located { root, path, relative })
}

/// The versions git keeps of a file with an unresolved merge conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictStages {
    /// The file on disk, which holds the conflict markers.
    pub path: PathBuf,
    /// The common ancestor (stage 1); `None` if both sides added the file.
    pub base: Option<Vec<u8>>,
    /// The checked-out side (stage 2); `None` if it deleted the file.
    pub ours: Option<Vec<u8>>,
    /// The side being merged in (stage 3); `None` if it deleted the file.
    pub theirs: Option<Vec<u8>>,
    /// Short hash of `HEAD`.
    pub ours_commit: Option<String>,
    /// The operation's ref (`MERGE_HEAD`, `REBASE_HEAD`, ...) and its short
    /// hash, if one is in progress.
    pub theirs_commit: Option<(&'static str, String)>,
}

/// Refs naming the incoming commit of an operation that can stop with
/// conflicts.
const INCOMING_REFS: [&str; 4] = ["MERGE_HEAD", "REBASE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"];

/// Returns true if `path` has an unresolved merge conflict. Errors (for
/// example a file outside any repository) count as no conflict.
pub fn has_conflict(path: &Path) -> bool {
    unmerged_entries(path).is_ok_and(|(_, entries)| !entries.is_empty())
}

/// Reads the stages of `path`'s merge conflict, or `None` if it has no
/// unresolved conflict.
pub fn conflict_stages(path: &Path) -> Result<Option<ConflictStages>, GitError> {
    let (located, entries) = unmerged_entries(path)?;
    if entries.is_empty() {
        return Ok(None);
    }
    let root = &located.root;
    let mut stages: [Option<Vec<u8>>; 3] = Default::default();
    for (stage, object) in entries {
        if let Some(slot) = stage.checked_sub(1).and_then(|i| stages.get_mut(i)) {
            *slot = Some(git(root, &["cat-file", "blob", &object])?);
        }
    }
    let short = |name: &str| {
        git(root, &["rev-parse", "-q", "--verify", "--short", name])
            .ok()
            .map(|out| String::from_utf8_lossy(&out).trim().to_owned())
            .filter(|hash| !hash.is_empty())
    };
    let ours_commit = short("HEAD");
    let theirs_commit = INCOMING_REFS.iter().find_map(|&name| short(name).map(|hash| (name, hash)));
    let [base, ours, theirs] = stages;
    Ok(Some(ConflictStages { path: located.path, base, ours, theirs, ours_commit, theirs_commit }))
}

/// The file's index entries in a conflict stage, as (stage, object id).
fn unmerged_entries(path: &Path) -> Result<(Located, Vec<(usize, String)>), GitError> {
    let located = locate(path)?;
    let relative = located.relative.to_string_lossy().into_owned();
    let output = git(&located.root, &["ls-files", "-u", "-z", "--", &relative])?;
    // Each record is "<mode> <object> <stage>\t<path>".
    let entries = output
        .split(|&byte| byte == 0)
        .filter_map(|record| {
            let record = String::from_utf8_lossy(record);
            let (info, _) = record.split_once('\t')?;
            let mut fields = info.split(' ');
            let (_mode, object, stage) = (fields.next()?, fields.next()?, fields.next()?);
            Some((stage.parse().ok()?, object.to_owned()))
        })
        .collect();
    Ok((located, entries))
}

pub(crate) fn git(folder: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(["-c", "core.quotepath=off", "--no-pager"])
        .args(args)
        .output()
        .map_err(GitError::GitUnavailable)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(GitError::Command(String::from_utf8_lossy(&output.stderr).trim().to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary repository with a committer configured.
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        run(dir.path(), &["init", "-q"]);
        run(dir.path(), &["config", "user.name", "Test"]);
        run(dir.path(), &["config", "user.email", "test@example.com"]);
        run(dir.path(), &["config", "commit.gpgsign", "false"]);
        dir
    }

    fn run(dir: &Path, args: &[&str]) {
        git(dir, args).expect("git command");
    }

    fn commit(dir: &Path, name: &str, text: &str, message: &str) {
        fs::write(dir.join(name), text).expect("write");
        run(dir, &["add", "-A"]);
        run(dir, &["commit", "-q", "-m", message]);
    }

    fn subjects(file: &GitFile) -> Vec<String> {
        file.versions()
            .iter()
            .map(|version| match version {
                FileVersion::Working => "working".to_owned(),
                FileVersion::Commit(commit) => commit.subject.clone(),
                FileVersion::Empty => "empty".to_owned(),
            })
            .collect()
    }

    /// A repository where merging branch `other` conflicts in `a.txt`.
    fn conflicted_repo() -> tempfile::TempDir {
        let dir = repo();
        commit(dir.path(), "a.txt", "one\ntwo\nthree\n", "base");
        run(dir.path(), &["checkout", "-q", "-b", "other"]);
        commit(dir.path(), "a.txt", "one\nTHEIRS\nthree\n", "theirs");
        run(dir.path(), &["checkout", "-q", "-"]);
        commit(dir.path(), "a.txt", "one\nOURS\nthree\n", "ours");
        // The merge stops with a conflict, which git reports as failure.
        assert!(git(dir.path(), &["merge", "-q", "other"]).is_err());
        dir
    }

    #[test]
    fn reads_the_stages_of_a_conflicted_file() {
        let dir = conflicted_repo();
        let path = dir.path().join("a.txt");
        assert!(has_conflict(&path));
        let stages = conflict_stages(&path).expect("stages").expect("conflict");
        assert_eq!(stages.base.as_deref(), Some(&b"one\ntwo\nthree\n"[..]));
        assert_eq!(stages.ours.as_deref(), Some(&b"one\nOURS\nthree\n"[..]));
        assert_eq!(stages.theirs.as_deref(), Some(&b"one\nTHEIRS\nthree\n"[..]));
        assert!(stages.ours_commit.is_some());
        assert_eq!(stages.theirs_commit.as_ref().map(|(name, _)| *name), Some("MERGE_HEAD"));
    }

    #[test]
    fn resolved_and_clean_files_have_no_conflict() {
        let dir = conflicted_repo();
        let path = dir.path().join("a.txt");
        fs::write(&path, "resolved\n").expect("write");
        run(dir.path(), &["add", "a.txt"]);
        assert!(!has_conflict(&path));
        assert_eq!(conflict_stages(&path).expect("stages"), None);
        let outside = tempfile::tempdir().expect("temp dir");
        let loose = outside.path().join("loose.txt");
        fs::write(&loose, "x").expect("write");
        assert!(!has_conflict(&loose));
    }

    #[test]
    fn clean_file_compares_with_the_previous_commit() {
        let dir = repo();
        commit(dir.path(), "a.txt", "one\n", "first");
        commit(dir.path(), "a.txt", "two\n", "second");
        let file = GitFile::open(&dir.path().join("a.txt")).expect("history");
        assert_eq!(subjects(&file), ["working", "second", "first", "empty"]);
        assert!(!file.has_uncommitted_changes());
        assert_eq!(file.initial_pair(), (2, 0));
        assert_eq!(file.read(2).expect("read"), b"one\n");
        assert_eq!(file.read(3).expect("read"), b"");
    }

    #[test]
    fn uncommitted_changes_compare_with_the_last_commit() {
        let dir = repo();
        commit(dir.path(), "a.txt", "one\n", "first");
        fs::write(dir.path().join("a.txt"), "edited\n").expect("write");
        let file = GitFile::open(&dir.path().join("a.txt")).expect("history");
        assert!(file.has_uncommitted_changes());
        assert_eq!(file.initial_pair(), (1, 0));
        assert_eq!(file.read(0).expect("read"), b"edited\n");
        assert_eq!(file.read(1).expect("read"), b"one\n");
    }

    #[test]
    fn a_single_commit_compares_with_empty() {
        let dir = repo();
        commit(dir.path(), "a.txt", "one\n", "first");
        let file = GitFile::open(&dir.path().join("a.txt")).expect("history");
        assert_eq!(file.initial_pair(), (2, 0));
        assert_eq!(file.versions()[2], FileVersion::Empty);
    }

    #[test]
    fn untracked_files_compare_with_empty() {
        let dir = repo();
        commit(dir.path(), "other.txt", "x\n", "first");
        fs::write(dir.path().join("new.txt"), "new\n").expect("write");
        let file = GitFile::open(&dir.path().join("new.txt")).expect("history");
        assert_eq!(subjects(&file), ["working", "empty"]);
        assert_eq!(file.initial_pair(), (1, 0));
    }

    #[test]
    fn renamed_files_read_old_contents_from_the_old_path() {
        let dir = repo();
        commit(dir.path(), "old.txt", "original\n", "create");
        run(dir.path(), &["mv", "old.txt", "new.txt"]);
        run(dir.path(), &["commit", "-q", "-m", "rename"]);
        let file = GitFile::open(&dir.path().join("new.txt")).expect("history");
        assert_eq!(subjects(&file), ["working", "rename", "create", "empty"]);
        assert_eq!(file.read(2).expect("read"), b"original\n");
    }

    #[test]
    fn blames_each_line_of_a_version() {
        let dir = repo();
        commit(dir.path(), "a.txt", "one\ntwo\n", "first");
        commit(dir.path(), "a.txt", "one\nTWO\nthree\n", "second");
        fs::write(dir.path().join("a.txt"), "one\nTWO\nthree\nfour\n").expect("write");
        let file = GitFile::open(&dir.path().join("a.txt")).expect("history");
        let summaries = |index: usize| -> Vec<String> {
            file.blame(index)
                .expect("blame")
                .into_iter()
                .map(|line| if line.uncommitted { "uncommitted".to_owned() } else { line.summary })
                .collect()
        };
        assert_eq!(summaries(0), ["first", "second", "second", "uncommitted"]);
        assert_eq!(summaries(1), ["first", "second", "second"]);
        assert_eq!(summaries(2), ["first", "first"]);
        assert!(summaries(3).is_empty());
        let line = &file.blame(1).expect("blame")[1];
        assert_eq!((line.author.as_str(), line.short_hash.len()), ("Test", 7));
    }

    #[test]
    fn files_outside_a_repository_are_reported() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("loose.txt"), "x").expect("write");
        let error = GitFile::open(&dir.path().join("loose.txt")).expect_err("no repo");
        assert!(matches!(error, GitError::NotInRepository(_)), "{error}");
    }
}
