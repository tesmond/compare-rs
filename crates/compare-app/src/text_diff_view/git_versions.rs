//! Browsing a file's git history: each side shows one version of the file
//! and can step to older or newer versions. The left side is always the
//! older of the two.

use std::sync::Arc;
use std::time::SystemTime;

use gpui::{Context, Task};

use compare_core::{BlameLine, DiffSide, FileVersion, GitFile, TextDocument};

use super::TextDiffView;
use crate::crash_log;
use crate::format;

/// Which way to step through a file's versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Older,
    Newer,
}

/// The `git blame` of the version shown on one side.
enum Blame {
    NotLoaded,
    Loading(#[allow(dead_code)] Task<()>),
    Ready(Arc<Vec<BlameLine>>),
    Failed,
}

/// The file whose history is shown, and the version on each side.
pub struct GitState {
    file: Arc<GitFile>,
    /// Index into the file's versions shown on each side (left, right).
    shown: [usize; 2],
    /// The version being loaded, so a newer request replaces it.
    loading: Option<Task<()>>,
    /// Blame of the version on each side, loaded on demand.
    blame: [Blame; 2],
}

impl TextDiffView {
    /// Shows `file`'s history, with version `shown[0]` on the left and
    /// `shown[1]` on the right (indices into the file's versions; the left
    /// must be the older).
    pub fn with_git_history(mut self, file: Arc<GitFile>, shown: [usize; 2]) -> Self {
        for side in [DiffSide::Left, DiffSide::Right] {
            let index = shown[side.index()];
            let size = self.side(side).info.size;
            self.describe_version(side, &file, index, size);
        }
        self.git = Some(GitState {
            file,
            shown,
            loading: None,
            blame: [Blame::NotLoaded, Blame::NotLoaded],
        });
        self
    }

    /// Returns true if the comparison shows a file's git history.
    pub fn is_git_history(&self) -> bool {
        self.git.is_some()
    }

    /// Sets the file details, save location, and read-only state of `side`
    /// to match the version it shows.
    fn describe_version(&mut self, side: DiffSide, file: &GitFile, index: usize, size: u64) {
        let info = file.version_info(index, size);
        let read_only = match &file.versions()[index] {
            FileVersion::Working => None,
            FileVersion::Commit(_) | FileVersion::Empty => Some("Read-only"),
        };
        let state = self.side_mut(side);
        let working = matches!(file.versions()[index], FileVersion::Working);
        state.path = working.then(|| file.path().to_path_buf());
        state.disk_modified = if working { info.modified } else { None };
        state.info = info;
        if let Some(reason) = read_only {
            state.read_only = Some(reason.into());
        }
    }

    /// The version `side` would show after a step, if there is one. The
    /// left side stays older than the right.
    pub(super) fn step_target(&self, side: DiffSide, step: Step) -> Option<usize> {
        let git = self.git.as_ref()?;
        let [left, right] = git.shown;
        let last = git.file.versions().len() - 1;
        match (side, step) {
            (DiffSide::Left, Step::Older) => (left < last).then(|| left + 1),
            (DiffSide::Left, Step::Newer) => (left - 1 > right).then(|| left - 1),
            (DiffSide::Right, Step::Older) => (right + 1 < left).then(|| right + 1),
            (DiffSide::Right, Step::Newer) => right.checked_sub(1),
        }
    }

    /// Why `side` cannot step to another version right now, if it cannot.
    pub(super) fn step_blocked(&self, side: DiffSide) -> Option<&'static str> {
        self.side(side)
            .is_modified()
            .then_some("Save or revert your changes before viewing another version")
    }

    /// Shows the next older or newer version on `side`.
    pub(super) fn step_version(&mut self, side: DiffSide, step: Step, cx: &mut Context<Self>) {
        let Some(target) = self.step_target(side, step) else {
            return;
        };
        if self.step_blocked(side).is_some() {
            return;
        }
        let Some(git) = &mut self.git else {
            return;
        };
        let file = git.file.clone();
        git.loading = Some(cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn({
                    let file = file.clone();
                    async move { file.read(target) }
                })
                .await;
            this.update(cx, |this, cx| match read {
                Ok(bytes) => {
                    let document = TextDocument::decode(&bytes);
                    let info = file.version_info(target, bytes.len() as u64);
                    this.replace_side(side, &document, file.path(), info, cx);
                    this.describe_version(side, &file, target, bytes.len() as u64);
                    if let Some(git) = &mut this.git {
                        git.shown[side.index()] = target;
                        git.loading = None;
                        git.blame[side.index()] = Blame::NotLoaded;
                    }
                    cx.notify();
                }
                Err(error) => {
                    crash_log::record_error("Reading a git version", &error.to_string());
                }
            })
            .ok();
        }));
    }

    /// Starts loading the blame of any side that does not have it yet.
    pub(super) fn ensure_blame(&mut self, cx: &mut Context<Self>) {
        let Some(git) = &mut self.git else {
            return;
        };
        for side in [DiffSide::Left, DiffSide::Right] {
            if !matches!(git.blame[side.index()], Blame::NotLoaded) {
                continue;
            }
            let (file, index) = (git.file.clone(), git.shown[side.index()]);
            let task = cx.spawn(async move |this, cx| {
                let blame = cx
                    .background_executor()
                    .spawn({
                        let file = file.clone();
                        async move { file.blame(index) }
                    })
                    .await;
                this.update(cx, |this, cx| {
                    let Some(git) = &mut this.git else {
                        return;
                    };
                    // Ignore a blame for a version the side no longer shows.
                    if git.shown[side.index()] != index {
                        return;
                    }
                    git.blame[side.index()] = match blame {
                        Ok(lines) => Blame::Ready(Arc::new(lines)),
                        Err(error) => {
                            crash_log::record_error("Reading git blame", &error.to_string());
                            Blame::Failed
                        }
                    };
                    cx.notify();
                })
                .ok();
            });
            git.blame[side.index()] = Blame::Loading(task);
        }
    }

    /// Drops the blame of `side` so it is read again (after saving it).
    pub(super) fn forget_blame(&mut self, side: DiffSide) {
        if let Some(git) = &mut self.git {
            git.blame[side.index()] = Blame::NotLoaded;
        }
    }

    /// The blame tooltip for `line` on `side`: who last changed it, when,
    /// and why.
    pub(super) fn blame_hint(&self, side: DiffSide, line: usize) -> Option<String> {
        let git = self.git.as_ref()?;
        if self.side(side).is_modified() {
            return Some("Blame describes the saved file; save to see it for edited lines".into());
        }
        match &git.blame[side.index()] {
            Blame::Ready(lines) => lines.get(line).map(describe_blame),
            Blame::NotLoaded | Blame::Loading(_) => Some("Loading blame…".to_owned()),
            Blame::Failed => None,
        }
    }

    /// A one-line description of the version on `side`, shown under the
    /// file name.
    pub(super) fn version_label(&self, side: DiffSide) -> Option<String> {
        let git = self.git.as_ref()?;
        Some(describe(&git.file, git.shown[side.index()]))
    }

    /// The tooltip for stepping `side`: what it would show next, or why it
    /// cannot step.
    pub(super) fn step_hint(&self, side: DiffSide, step: Step) -> String {
        let direction = match step {
            Step::Older => "Older",
            Step::Newer => "Newer",
        };
        if let Some(reason) = self.step_blocked(side) {
            return reason.to_owned();
        }
        match (self.step_target(side, step), &self.git) {
            (Some(target), Some(git)) => format!("{direction}: {}", describe(&git.file, target)),
            _ => format!("No {} changes", direction.to_lowercase()),
        }
    }
}

/// Describes who last changed a line, e.g.
/// `a1b2c3d · Ann · 3 days ago (2026-11-03 14:02)` and the commit subject.
fn describe_blame(line: &BlameLine) -> String {
    if line.uncommitted {
        return "Not committed yet".to_owned();
    }
    let age = SystemTime::now()
        .duration_since(line.time)
        .map(|age| format!("{} ago", format::human_duration(age)))
        .unwrap_or_else(|_| "in the future".to_owned());
    format!(
        "{} · {} · {age} ({})\n{}",
        line.short_hash,
        line.author,
        format::format_time(Some(line.time)),
        line.summary
    )
}

/// Describes version `index` of `file`, e.g. `a1b2c3d · Fix parsing · Ann`.
fn describe(file: &GitFile, index: usize) -> String {
    match &file.versions()[index] {
        FileVersion::Working if file.has_uncommitted_changes() => {
            "Working copy · uncommitted changes".to_owned()
        }
        FileVersion::Working => "Working copy".to_owned(),
        FileVersion::Commit(commit) => {
            format!("{} · {} · {}", commit.short_hash, commit.subject, commit.author)
        }
        FileVersion::Empty => "Empty · the file did not exist yet".to_owned(),
    }
}
