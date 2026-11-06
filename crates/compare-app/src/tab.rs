//! A comparison tab and its state.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{Entity, SharedString, Subscription};

use compare_core::{CompareTarget, FolderProgress};

use crate::folder_diff_view::FolderDiffView;
use crate::hex_diff_view::HexDiffView;
use crate::merge_view::MergeView;
use crate::text_diff_view::TextDiffView;

/// Whether a tab compares two items or merges three files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Compare,
    Merge,
    /// Compares text pasted into the app rather than files.
    Text,
}

/// One input slot of a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Base,
    Right,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Base => "base",
            Side::Right => "right",
        }
    }
}

/// What a tab is currently showing.
pub enum TabContent {
    /// Waiting for the user to choose inputs.
    Empty,
    /// A comparison is running; folder comparisons report progress.
    Loading {
        progress: Option<Arc<FolderProgress>>,
    },
    Text(Entity<TextDiffView>),
    Binary(Entity<HexDiffView>),
    Folder(Entity<FolderDiffView>),
    Merge(Entity<MergeView>),
    Error(SharedString),
}

/// A comparison tab.
pub struct Tab {
    pub id: u64,
    pub kind: TabKind,
    pub left: Option<PathBuf>,
    pub base: Option<PathBuf>,
    pub right: Option<PathBuf>,
    pub content: TabContent,
    /// Incremented for every comparison started, so stale results are dropped.
    pub generation: u64,
    /// Keeps extracted archives alive while their comparison is shown.
    /// Shared with tabs opened from this one, so extracted archives stay
    /// on disk until every tab using them has closed.
    pub targets: Vec<Arc<CompareTarget>>,
    pub subscription: Option<Subscription>,
}

impl Tab {
    pub fn new(id: u64, kind: TabKind) -> Self {
        Self {
            id,
            kind,
            left: None,
            base: None,
            right: None,
            content: TabContent::Empty,
            generation: 0,
            targets: Vec::new(),
            subscription: None,
        }
    }

    pub fn path(&self, side: Side) -> Option<&Path> {
        match side {
            Side::Left => self.left.as_deref(),
            Side::Base => self.base.as_deref(),
            Side::Right => self.right.as_deref(),
        }
    }

    pub fn set_path(&mut self, side: Side, path: PathBuf) {
        match side {
            Side::Left => self.left = Some(path),
            Side::Base => self.base = Some(path),
            Side::Right => self.right = Some(path),
        }
    }

    /// The input slots this tab uses, in display order.
    pub fn sides(&self) -> &'static [Side] {
        match self.kind {
            TabKind::Compare => &[Side::Left, Side::Right],
            TabKind::Merge => &[Side::Left, Side::Base, Side::Right],
            TabKind::Text => &[],
        }
    }

    /// Returns true when every input slot is filled.
    pub fn is_ready(&self) -> bool {
        self.sides().iter().all(|&side| self.path(side).is_some())
    }

    /// The first unfilled input slot.
    pub fn first_empty_side(&self) -> Option<Side> {
        self.sides().iter().copied().find(|&side| self.path(side).is_none())
    }

    pub fn title(&self) -> String {
        let name = |path: Option<&Path>| {
            path.and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned())
        };
        if self.kind == TabKind::Text {
            return "Text Compare".to_owned();
        }
        let prefix = if self.kind == TabKind::Merge { "Merge: " } else { "" };
        match (name(self.left.as_deref()), name(self.right.as_deref())) {
            (None, None) if self.kind == TabKind::Merge => "New Merge".to_owned(),
            (None, None) => "New Comparison".to_owned(),
            (Some(left), Some(right)) if left == right => format!("{prefix}{left}"),
            (Some(left), Some(right)) => format!("{prefix}{left} ↔ {right}"),
            (Some(one), None) | (None, Some(one)) => format!("{prefix}{one} ↔ …"),
        }
    }

    /// Stops any running folder comparison.
    pub fn cancel(&self) {
        if let TabContent::Loading { progress: Some(progress) } = &self.content {
            progress.cancel();
        }
    }
}
