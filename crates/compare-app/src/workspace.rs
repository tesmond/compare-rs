//! The main window: tabs, toolbar, path bar, content, and status bar.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext, ClickEvent, ClipboardItem, Context, Div, Entity, ExternalPaths, FocusHandle,
    Focusable, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, PathPromptOptions,
    PromptLevel, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    Window, div, px,
};
use gpui_component::{Icon, IconName, Sizable, tooltip::Tooltip};

use compare_core::{
    CompareOptions, CompareTarget, ComparisonContent, DiffOptions, FileComparison, FileInfo,
    FolderComparison, FolderProgress, GitFile, MergeDocument, TargetKind, TextDiff, TextDocument,
};

use crate::actions::{
    Beautify, CloseTab, Compare, CompareFolders, CopyPaths, FindNext, FindPrev, Merge, NewTab,
    NewTextCompare, NextDifference, NextTab, OpenFind, OpenFolderLeft, OpenFolderRight,
    OpenGitHistory, OpenLeft, OpenReplace, OpenRight, PrevDifference, PrevTab, Quit, RevertFile,
    SaveAll, SaveFile, ToggleIgnoreCase, ToggleIgnoreWhitespace, ToggleWordWrap,
};
use crate::app_assets;
use crate::button::{self, ButtonState, Content, Segment};
use crate::crash_log;
use crate::document_syntax::DocumentSyntax;
use crate::folder_diff_view::{FolderDiffView, OpenFilePair};
use crate::format;
use crate::hex_diff_view::HexDiffView;
use crate::merge_view::MergeView;
use crate::session_state::{self, SavedBounds, SavedTab, SessionState};
use crate::tab::{Side, Tab, TabContent, TabKind};
use crate::text_diff_view::{TextDiffEvent, TextDiffView};
use crate::theme;

/// How long the session waits after the last change before it is saved.
const SESSION_SAVE_DELAY: Duration = Duration::from_millis(500);

/// How often the UI refreshes while a folder comparison runs.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Whether a picker selects files or folders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickKind {
    File,
    Folder,
}

/// The finished result of a background comparison.
enum CompareOutcome {
    /// `syntax` holds the left and right syntax state for text comparisons.
    Files {
        comparison: FileComparison,
        syntax: [DocumentSyntax; 2],
    },
    Folders {
        comparison: FolderComparison,
        targets: Vec<CompareTarget>,
    },
    /// A file's git history, opened on its first two versions.
    History {
        file: Arc<GitFile>,
        /// Version indices shown on the left and right.
        shown: [usize; 2],
        infos: [FileInfo; 2],
        identical: bool,
        diff: TextDiff,
        syntax: [DocumentSyntax; 2],
    },
    /// `infos` and `syntax` are ordered left, base, right.
    Merge {
        infos: [FileInfo; 3],
        syntax: [DocumentSyntax; 3],
        document: MergeDocument,
    },
    Failed(String),
}

/// The root view of the application window.
pub struct Workspace {
    focus: FocusHandle,
    tabs: Vec<Tab>,
    active: usize,
    next_tab_id: u64,
    options: CompareOptions,
    /// Whether long lines wrap in text views.
    wrap: bool,
    /// Latest window placement, kept for the saved session.
    window_bounds: Option<SavedBounds>,
    /// The session as last saved, to detect changes.
    saved_session: Option<SessionState>,
    pending_save: Option<Task<()>>,
    _bounds_subscription: Option<Subscription>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Workspace {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 1,
            options: CompareOptions::default(),
            wrap: false,
            window_bounds: None,
            saved_session: None,
            pending_save: None,
            _bounds_subscription: None,
        }
    }

    // ─── Session ────────────────────────────────────────────────────────────

    /// Tracks the window's placement so it can be restored next launch.
    pub fn watch_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.window_bounds = Some(SavedBounds::from_window_bounds(window.window_bounds()));
        self._bounds_subscription = Some(cx.observe_window_bounds(window, |this, window, cx| {
            this.window_bounds = Some(SavedBounds::from_window_bounds(window.window_bounds()));
            cx.notify();
        }));
    }

    /// Reopens the tabs and options of a saved session.
    pub fn restore(&mut self, session: SessionState, cx: &mut Context<Self>) {
        self.options.diff.ignore_whitespace = session.ignore_whitespace;
        self.options.diff.ignore_case = session.ignore_case;
        self.wrap = session.word_wrap;
        for saved in session.tabs {
            let kind = if saved.merge { TabKind::Merge } else { TabKind::Compare };
            let id = self.add_tab(kind, cx);
            let inputs =
                [(Side::Left, saved.left), (Side::Base, saved.base), (Side::Right, saved.right)];
            for (side, path) in inputs {
                if let Some(path) = path {
                    self.set_tab_path(id, side, path, cx);
                }
            }
        }
        self.active = session.active_tab.min(self.tabs.len().saturating_sub(1));
        cx.notify();
    }

    /// The current session, as it would be saved.
    pub fn session(&self) -> SessionState {
        let tabs = self
            .tabs
            .iter()
            // Pasted text is not saved.
            .filter(|tab| matches!(tab.kind, TabKind::Compare | TabKind::Merge))
            .map(|tab| SavedTab {
                merge: tab.kind == TabKind::Merge,
                left: tab.left.clone(),
                base: tab.base.clone(),
                right: tab.right.clone(),
            })
            .collect();
        SessionState {
            window: self.window_bounds,
            tabs,
            active_tab: self.active,
            ignore_whitespace: self.options.diff.ignore_whitespace,
            ignore_case: self.options.diff.ignore_case,
            word_wrap: self.wrap,
        }
    }

    /// Saves the session shortly after it changes, coalescing bursts such as
    /// window drags into one write.
    fn save_session_if_changed(&mut self, cx: &mut Context<Self>) {
        let session = self.session();
        if self.saved_session.as_ref() == Some(&session) {
            return;
        }
        self.saved_session = Some(session.clone());
        self.pending_save = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SESSION_SAVE_DELAY).await;
            let result =
                cx.background_executor().spawn(async move { session_state::save(&session) }).await;
            if let Err(error) = result {
                crash_log::record_error("Saving session", &error.to_string());
            }
        }));
    }

    // ─── Tab management ─────────────────────────────────────────────────────

    fn add_tab(&mut self, kind: TabKind, cx: &mut Context<Self>) -> u64 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(Tab::new(id, kind));
        self.active = self.tabs.len() - 1;
        cx.notify();
        id
    }

    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    /// The editable comparison shown in the tab at `index`, if any.
    fn text_view(&self, index: usize) -> Option<&Entity<TextDiffView>> {
        match &self.tabs.get(index)?.content {
            TabContent::Text(view) => Some(view),
            _ => None,
        }
    }

    /// Returns true if the tab at `index` has unsaved edits.
    fn tab_is_modified(&self, index: usize, cx: &App) -> bool {
        self.text_view(index).is_some_and(|view| view.read(cx).is_modified())
    }

    /// Closes the tab at `index`, first asking whether to save any unsaved
    /// edits.
    fn request_close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.text_view(index).filter(|view| view.read(cx).is_modified()).cloned()
        else {
            self.close_tab(index, cx);
            return;
        };
        let names = view.read(cx).modified_names().join(" and ");
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Save changes to {names}?"),
            Some("Your changes will be lost if you don't save them."),
            &["Save", "Don't Save", "Cancel"],
            cx,
        );
        let tab_id = self.tabs[index].id;
        cx.spawn_in(window, async move |this, cx| {
            match answer.await {
                Ok(0) => {
                    let Ok(save) = view.update_in(cx, |view, window, cx| view.save_all(window, cx))
                    else {
                        return;
                    };
                    if !save.await {
                        return;
                    }
                }
                Ok(1) => {}
                _ => return,
            }
            this.update(cx, |this, cx| {
                if let Some(index) = this.tab_index(tab_id) {
                    this.close_tab(index, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Quits, first asking whether to save unsaved edits in any tab.
    pub fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let views: Vec<Entity<TextDiffView>> = (0..self.tabs.len())
            .filter(|&index| self.tab_is_modified(index, cx))
            .filter_map(|index| self.text_view(index).cloned())
            .collect();
        if views.is_empty() {
            cx.quit();
            return;
        }
        let names: Vec<String> =
            views.iter().flat_map(|view| view.read(cx).modified_names()).collect();
        let message = match names.as_slice() {
            [one] => format!("Save changes to {one} before quitting?"),
            many => format!("Save changes to {} files before quitting?", many.len()),
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some("Your changes will be lost if you don't save them."),
            &["Save All", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |_, cx| {
            match answer.await {
                Ok(0) => {
                    for view in views {
                        let Ok(save) =
                            view.update_in(cx, |view, window, cx| view.save_all(window, cx))
                        else {
                            return;
                        };
                        if !save.await {
                            return;
                        }
                    }
                }
                Ok(1) => {}
                _ => return,
            }
            cx.update(|_, cx| cx.quit()).ok();
        })
        .detach();
    }

    /// Returns true if the window may close now; otherwise asks about
    /// unsaved edits first (and quits once they are dealt with).
    pub fn confirm_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let modified = (0..self.tabs.len()).any(|index| self.tab_is_modified(index, cx));
        if modified {
            self.request_quit(window, cx);
        }
        !modified
    }

    fn on_save_file(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.text_view(self.active).cloned() {
            view.update(cx, |view, cx| view.save_active(window, cx)).detach();
        }
    }

    fn on_save_all(&mut self, _: &SaveAll, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.text_view(self.active).cloned() {
            view.update(cx, |view, cx| view.save_all(window, cx)).detach();
        }
    }

    /// Find shortcuts reach the active comparison even when it does not
    /// have keyboard focus.
    fn on_open_find(&mut self, _: &OpenFind, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.text_view(self.active).cloned() {
            view.update(cx, |view, cx| view.open_find(false, window, cx));
        }
    }

    fn on_open_replace(&mut self, _: &OpenReplace, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.text_view(self.active).cloned() {
            view.update(cx, |view, cx| view.open_find(true, window, cx));
        }
    }

    fn on_find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.text_view(self.active).cloned() {
            view.update(cx, |view, cx| view.find_step(true, cx));
        }
    }

    fn on_find_prev(&mut self, _: &FindPrev, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.text_view(self.active).cloned() {
            view.update(cx, |view, cx| view.find_step(false, cx));
        }
    }

    fn on_revert_file(&mut self, _: &RevertFile, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.text_view(self.active).cloned() {
            view.update(cx, |view, cx| view.revert_active(cx));
        }
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    /// Returns the active tab's id, creating a comparison tab if there is none.
    fn active_or_new_tab(&mut self, cx: &mut Context<Self>) -> u64 {
        match self.active_tab() {
            // Files chosen while a pasted-text tab is active open a new tab.
            Some(tab) if matches!(tab.kind, TabKind::Compare | TabKind::Merge) => tab.id,
            _ => self.add_tab(TabKind::Compare, cx),
        }
    }

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs[index].cancel();
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if index < self.active {
            self.active -= 1;
        }
        cx.notify();
    }

    /// Fills an input slot of a tab and starts comparing once all are filled.
    fn set_tab_path(&mut self, tab_id: u64, side: Side, path: PathBuf, cx: &mut Context<Self>) {
        let Some(mut index) = self.tab_index(tab_id) else {
            return;
        };
        if self.tab_is_modified(index, cx) {
            // Keep the unsaved edits: compare the new file in a new tab.
            let (kind, inputs) = {
                let tab = &self.tabs[index];
                (tab.kind, [tab.left.clone(), tab.base.clone(), tab.right.clone()])
            };
            let id = self.add_tab(kind, cx);
            index = self.tab_index(id).expect("tab was just added");
            let tab = &mut self.tabs[index];
            [tab.left, tab.base, tab.right] = inputs;
        }
        self.tabs[index].set_path(side, path);
        if self.tabs[index].is_ready() {
            self.start_compare(index, cx);
        }
        cx.notify();
    }

    /// Assigns dropped paths to the active tab: two paths fill both sides,
    /// a single path fills the first empty slot (or `side` if given).
    fn drop_paths(&mut self, paths: &[PathBuf], side: Option<Side>, cx: &mut Context<Self>) {
        let tab_id = self.active_or_new_tab(cx);
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let tab = &mut self.tabs[index];
        match (paths, side) {
            ([left, right], None) if tab.kind == TabKind::Compare => {
                tab.set_path(Side::Left, left.clone());
                tab.set_path(Side::Right, right.clone());
            }
            ([path, ..], Some(side)) => tab.set_path(side, path.clone()),
            ([path, ..], None) => {
                let side = tab.first_empty_side().unwrap_or(Side::Left);
                tab.set_path(side, path.clone());
            }
            ([], _) => return,
        }
        if self.tabs[index].is_ready() {
            self.start_compare(index, cx);
        }
        cx.notify();
    }

    // ─── Comparison pipeline ────────────────────────────────────────────────

    /// Starts (or restarts) the comparison for the tab at `index` on a
    /// background thread.
    fn start_compare(&mut self, index: usize, cx: &mut Context<Self>) {
        let options = self.options;
        let tab = &mut self.tabs[index];
        tab.cancel();
        tab.generation += 1;
        tab.subscription = None;
        let (tab_id, generation, kind) = (tab.id, tab.generation, tab.kind);
        if kind == TabKind::History {
            let Some(file) = tab.right.clone() else {
                return;
            };
            tab.content = TabContent::Loading { progress: None };
            let task =
                cx.background_executor().spawn(async move { git_history(&file, &options.diff) });
            cx.spawn(async move |this, cx| {
                let outcome = task.await;
                this.update(cx, |this, cx| this.finish_compare(tab_id, generation, outcome, cx))
                    .ok();
            })
            .detach();
            cx.notify();
            return;
        }
        let (Some(left), Some(right)) = (tab.left.clone(), tab.right.clone()) else {
            return;
        };
        let base = tab.base.clone();

        let mut progress: Option<Arc<FolderProgress>> = None;
        let job: Box<dyn FnOnce() -> CompareOutcome + Send> = match (kind, base) {
            (TabKind::Merge, Some(base)) => {
                Box::new(move || merge_files(&left, &base, &right, &options.diff))
            }
            (TabKind::Merge, None) | (TabKind::Text | TabKind::History, _) => return,
            (TabKind::Compare, _) => match (TargetKind::of(&left), TargetKind::of(&right)) {
                (TargetKind::File, TargetKind::File) => {
                    Box::new(move || compare_file_pair(&left, &right, &options))
                }
                (l, r) if l.is_folder_like() && r.is_folder_like() => {
                    let shared = Arc::new(FolderProgress::default());
                    progress = Some(shared.clone());
                    Box::new(move || compare_folder_targets(&left, &right, &shared))
                }
                (TargetKind::Missing, _) => {
                    tab.content =
                        TabContent::Error(format!("{} does not exist", left.display()).into());
                    return;
                }
                (_, TargetKind::Missing) => {
                    tab.content =
                        TabContent::Error(format!("{} does not exist", right.display()).into());
                    return;
                }
                _ => {
                    tab.content = TabContent::Error(
                        "Cannot compare a file with a folder. Choose two files or two folders."
                            .into(),
                    );
                    return;
                }
            },
        };

        tab.content = TabContent::Loading { progress: progress.clone() };
        let task = cx.background_executor().spawn(async move { job() });
        cx.spawn(async move |this, cx| {
            let outcome = task.await;
            this.update(cx, |this, cx| this.finish_compare(tab_id, generation, outcome, cx)).ok();
        })
        .detach();

        if progress.is_some() {
            self.poll_progress(tab_id, generation, cx);
        }
        cx.notify();
    }

    /// Re-renders periodically while a folder comparison is loading.
    fn poll_progress(&mut self, tab_id: u64, generation: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PROGRESS_INTERVAL).await;
                let still_loading = this
                    .update(cx, |this, cx| {
                        let loading = this.tab_index(tab_id).is_some_and(|i| {
                            let tab = &this.tabs[i];
                            tab.generation == generation
                                && matches!(tab.content, TabContent::Loading { .. })
                        });
                        if loading {
                            cx.notify();
                        }
                        loading
                    })
                    .unwrap_or(false);
                if !still_loading {
                    break;
                }
            }
        })
        .detach();
    }

    fn finish_compare(
        &mut self,
        tab_id: u64,
        generation: u64,
        outcome: CompareOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if self.tabs[index].generation != generation {
            return;
        }
        let content = match outcome {
            CompareOutcome::Files { comparison, syntax } => {
                let FileComparison { left, right, identical, content } = comparison;
                let [left_syntax, right_syntax] = syntax;
                match content {
                    ComparisonContent::Text(diff) => {
                        let (options, wrap) = (self.options.diff, self.wrap);
                        let in_archive = |path: &Path| {
                            self.tabs[index]
                                .targets
                                .iter()
                                .any(|target| {
                                    target.is_extracted() && path.starts_with(target.folder())
                                })
                                .then_some("Read-only: inside an archive")
                        };
                        let read_only = [in_archive(&left.path), in_archive(&right.path)];
                        let view = cx.new(|cx| {
                            TextDiffView::new(
                                left,
                                right,
                                identical,
                                diff,
                                left_syntax,
                                right_syntax,
                                options,
                                cx,
                            )
                            .with_wrap(wrap)
                            .with_read_only(read_only)
                        });
                        self.tabs[index].subscription =
                            Some(self.subscribe_to_text_view(&view, cx));
                        TabContent::Text(view)
                    }
                    ComparisonContent::Binary(diff) => TabContent::Binary(
                        cx.new(|cx| HexDiffView::new(left, right, identical, diff, cx)),
                    ),
                }
            }
            CompareOutcome::Folders { comparison, targets } => {
                let labels: Vec<String> =
                    targets.iter().map(|t| format::path_label(t.original())).collect();
                let (left_label, right_label) = (labels[0].clone(), labels[1].clone());
                let view =
                    cx.new(|cx| FolderDiffView::new(comparison, left_label, right_label, cx));
                let subscription = cx.subscribe(&view, |this, folder, event: &OpenFilePair, cx| {
                    let targets = this
                        .tabs
                        .iter()
                        .find(|tab| matches!(&tab.content, TabContent::Folder(v) if *v == folder))
                        .map(|tab| tab.targets.clone())
                        .unwrap_or_default();
                    this.open_file_pair(event.left.clone(), event.right.clone(), targets, cx);
                });
                let tab = &mut self.tabs[index];
                tab.targets = targets.into_iter().map(Arc::new).collect();
                tab.subscription = Some(subscription);
                TabContent::Folder(view)
            }
            CompareOutcome::History { file, shown, infos, identical, diff, syntax } => {
                let (options, wrap) = (self.options.diff, self.wrap);
                let [left, right] = infos;
                let [left_syntax, right_syntax] = syntax;
                let view = cx.new(|cx| {
                    TextDiffView::new(
                        left,
                        right,
                        identical,
                        diff,
                        left_syntax,
                        right_syntax,
                        options,
                        cx,
                    )
                    .with_wrap(wrap)
                    .with_git_history(file, shown)
                });
                self.tabs[index].subscription = Some(self.subscribe_to_text_view(&view, cx));
                TabContent::Text(view)
            }
            CompareOutcome::Merge { infos, syntax, document } => {
                let wrap = self.wrap;
                TabContent::Merge(
                    cx.new(|cx| MergeView::new(infos, syntax, document, cx).with_wrap(wrap)),
                )
            }
            CompareOutcome::Failed(message) => {
                crash_log::record_error("Comparison failed", &message);
                TabContent::Error(message.into())
            }
        };
        self.tabs[index].content = content;
        cx.notify();
    }

    /// Opens a file pair from a folder comparison in a new tab.
    fn open_file_pair(
        &mut self,
        left: PathBuf,
        right: PathBuf,
        targets: Vec<Arc<CompareTarget>>,
        cx: &mut Context<Self>,
    ) {
        let id = self.add_tab(TabKind::Compare, cx);
        if let Some(index) = self.tab_index(id) {
            self.tabs[index].targets = targets;
        }
        self.set_tab_path(id, Side::Left, left, cx);
        self.set_tab_path(id, Side::Right, right, cx);
    }

    /// Opens paths given on the command line: two paths are compared, three
    /// are merged as `LEFT BASE RIGHT`, and `--history FILE` opens the
    /// file's git history.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if let [flag, file] = paths.as_slice()
            && flag.as_os_str() == "--history"
        {
            self.open_git_history(file.clone(), cx);
            return;
        }
        let (kind, sides): (TabKind, &[Side]) = match paths.len() {
            2 => (TabKind::Compare, &[Side::Left, Side::Right]),
            3 => (TabKind::Merge, &[Side::Left, Side::Base, Side::Right]),
            _ => return,
        };
        let id = self.add_tab(kind, cx);
        for (&side, path) in sides.iter().zip(paths) {
            self.set_tab_path(id, side, path, cx);
        }
    }

    // ─── Pickers ────────────────────────────────────────────────────────────

    fn pick_path(&mut self, side: Side, kind: PickKind, cx: &mut Context<Self>) {
        let tab_id = self.active_or_new_tab(cx);
        let receiver = cx.prompt_for_paths(prompt_options(side, kind));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            if let Some(path) = paths.into_iter().next() {
                this.update(cx, |this, cx| this.set_tab_path(tab_id, side, path, cx)).ok();
            }
        })
        .detach();
    }

    /// Prompts, in order, for every merge input the tab is still missing.
    fn prompt_merge_inputs(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            for side in [Side::Left, Side::Base, Side::Right] {
                let missing = this
                    .update(cx, |this, _| {
                        this.tab_index(tab_id).is_some_and(|i| this.tabs[i].path(side).is_none())
                    })
                    .unwrap_or(false);
                if !missing {
                    continue;
                }
                let Ok(receiver) =
                    cx.update(|cx| cx.prompt_for_paths(prompt_options(side, PickKind::File)))
                else {
                    return;
                };
                let Ok(Ok(Some(paths))) = receiver.await else {
                    return;
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                this.update(cx, |this, cx| this.set_tab_path(tab_id, side, path, cx)).ok();
            }
        })
        .detach();
    }

    // ─── Action handlers ────────────────────────────────────────────────────

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.add_tab(TabKind::Compare, cx);
    }

    fn on_open_left(&mut self, _: &OpenLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(Side::Left, PickKind::File, cx);
    }

    fn on_open_right(&mut self, _: &OpenRight, _: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(Side::Right, PickKind::File, cx);
    }

    fn on_open_folder_left(&mut self, _: &OpenFolderLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(Side::Left, PickKind::Folder, cx);
    }

    fn on_open_folder_right(
        &mut self,
        _: &OpenFolderRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pick_path(Side::Right, PickKind::Folder, cx);
    }

    fn on_compare(&mut self, _: &Compare, _: &mut Window, cx: &mut Context<Self>) {
        self.compare_active(cx);
    }

    fn on_compare_folders(&mut self, _: &CompareFolders, _: &mut Window, cx: &mut Context<Self>) {
        self.compare_active(cx);
    }

    fn compare_active(&mut self, cx: &mut Context<Self>) {
        if self.tab_is_modified(self.active, cx) {
            // Edited text is re-diffed as it changes; re-reading the files
            // would discard the edits.
            return;
        }
        if let Some(tab) = self.active_tab() {
            if tab.is_ready() {
                self.start_compare(self.active, cx);
            } else if let Some(side) = tab.first_empty_side() {
                let kind = match side {
                    Side::Base => PickKind::File,
                    _ if tab.path(Side::Left).is_some_and(|p| p.is_dir()) => PickKind::Folder,
                    _ => PickKind::File,
                };
                self.pick_path(side, kind, cx);
            }
        }
    }

    fn on_merge(&mut self, _: &Merge, _: &mut Window, cx: &mut Context<Self>) {
        // Seed the merge with the active comparison's files, if any.
        let seed = self.active_tab().filter(|tab| tab.kind == TabKind::Compare).map(|tab| {
            let file = |path: Option<&Path>| path.filter(|p| p.is_file()).map(Path::to_path_buf);
            (file(tab.path(Side::Left)), file(tab.path(Side::Right)))
        });
        let tab_id = self.add_tab(TabKind::Merge, cx);
        if let (Some((left, right)), Some(index)) = (seed, self.tab_index(tab_id)) {
            let tab = &mut self.tabs[index];
            tab.left = left;
            tab.right = right;
        }
        self.prompt_merge_inputs(tab_id, cx);
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.request_close_tab(self.active, window, cx);
    }

    fn on_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
            cx.notify();
        }
        self.focus_active_tab(window, cx);
    }

    fn on_prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            cx.notify();
        }
        self.focus_active_tab(window, cx);
    }

    fn on_next_difference(&mut self, _: &NextDifference, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate_difference(true, cx);
    }

    fn on_prev_difference(&mut self, _: &PrevDifference, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate_difference(false, cx);
    }

    fn navigate_difference(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        match &tab.content {
            TabContent::Text(view) => view.update(cx, |view, cx| {
                if forward { view.next_difference(cx) } else { view.prev_difference(cx) }
            }),
            TabContent::Binary(view) => view.update(cx, |view, cx| {
                if forward { view.next_difference(cx) } else { view.prev_difference(cx) }
            }),
            TabContent::Folder(view) => view.update(cx, |view, cx| {
                if forward { view.next_difference(cx) } else { view.prev_difference(cx) }
            }),
            TabContent::Merge(view) => view.update(cx, |view, cx| {
                if forward { view.next_difference(cx) } else { view.prev_difference(cx) }
            }),
            TabContent::Empty | TabContent::Loading { .. } | TabContent::Error(_) => {}
        }
    }

    fn on_new_text_compare(
        &mut self,
        _: &NewTextCompare,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (options, wrap) = (self.options.diff, self.wrap);
        let untitled = || FileInfo { path: PathBuf::from("Untitled"), size: 0, modified: None };
        let view = cx.new(|cx| {
            let empty = TextDocument::default();
            let diff = TextDiff::new(empty.clone(), empty, &options);
            let syntax = DocumentSyntax::default();
            TextDiffView::new(
                untitled(),
                untitled(),
                true,
                diff,
                syntax.clone(),
                syntax,
                options,
                cx,
            )
            .without_file_paths()
            .with_wrap(wrap)
        });
        let id = self.add_tab(TabKind::Text, cx);
        if let Some(index) = self.tab_index(id) {
            self.tabs[index].subscription = Some(self.subscribe_to_text_view(&view, cx));
            self.tabs[index].content = TabContent::Text(view);
        }
        self.focus_active_tab(window, cx);
        cx.notify();
    }

    /// Reformats JSON, Python dicts, and `\n` escapes on both sides of the
    /// active comparison, as an edit that can be undone.
    /// Asks for a file and opens its git history in a new tab.
    fn on_open_git_history(&mut self, _: &OpenGitHistory, _: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Show History".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            if let Some(path) = paths.into_iter().next() {
                this.update(cx, |this, cx| this.open_git_history(path, cx)).ok();
            }
        })
        .detach();
    }

    /// Opens the git history of `path` in a new tab: the file on the right,
    /// and an earlier version on the left.
    pub fn open_git_history(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let id = self.add_tab(TabKind::History, cx);
        if let Some(index) = self.tab_index(id) {
            self.tabs[index].right = Some(path);
            self.start_compare(index, cx);
        }
    }

    fn on_beautify(&mut self, _: &Beautify, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.text_view(self.active).cloned() {
            view.update(cx, |view, cx| view.beautify(cx));
        }
    }

    /// Gives keyboard focus to the active tab's editor, or else the
    /// workspace.
    fn focus_active_tab(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.text_view(self.active) {
            Some(view) => window.focus(&view.focus_handle(cx)),
            None => window.focus(&self.focus),
        }
    }

    /// Reacts to saves in a text comparison.
    fn subscribe_to_text_view(
        &self,
        view: &Entity<TextDiffView>,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe(view, |this, _, event: &TextDiffEvent, cx| match event {
            TextDiffEvent::Saved(path) => this.file_saved(path, cx),
        })
    }

    /// Updates folder comparisons that include a file that was just saved.
    fn file_saved(&mut self, path: &Path, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            if let TabContent::Folder(view) = &tab.content {
                view.update(cx, |view, cx| view.refresh_file(path, cx));
            }
        }
        cx.notify();
    }

    fn on_toggle_ignore_whitespace(
        &mut self,
        _: &ToggleIgnoreWhitespace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.options.diff.ignore_whitespace = !self.options.diff.ignore_whitespace;
        self.recompare_text_tabs(cx);
    }

    fn on_toggle_ignore_case(
        &mut self,
        _: &ToggleIgnoreCase,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.options.diff.ignore_case = !self.options.diff.ignore_case;
        self.recompare_text_tabs(cx);
    }

    fn on_toggle_word_wrap(
        &mut self,
        _: &ToggleWordWrap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.wrap = !self.wrap;
        let wrap = self.wrap;
        for tab in &self.tabs {
            match &tab.content {
                TabContent::Text(view) => view.update(cx, |view, cx| view.set_wrap(wrap, cx)),
                TabContent::Merge(view) => {
                    view.update(cx, |view, cx| view.set_wrap(wrap, window, cx));
                }
                _ => {}
            }
        }
        cx.notify();
    }

    /// Re-runs every comparison whose result depends on the diff options.
    /// Text comparisons are re-diffed in place, keeping any edits.
    fn recompare_text_tabs(&mut self, cx: &mut Context<Self>) {
        let options = self.options;
        for index in 0..self.tabs.len() {
            match &self.tabs[index].content {
                TabContent::Text(view) => {
                    view.update(cx, |view, cx| view.set_options(options.diff, cx));
                }
                TabContent::Merge(_) => self.start_compare(index, cx),
                _ => {}
            }
        }
        cx.notify();
    }

    fn on_copy_paths(&mut self, _: &CopyPaths, _: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let paths: Vec<PathBuf> = match &tab.content {
            TabContent::Folder(view) => view.read(cx).selected_paths(),
            _ => tab
                .sides()
                .iter()
                .filter_map(|&side| tab.path(side).map(Path::to_path_buf))
                .collect(),
        };
        let text = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    // ─── Rendering ──────────────────────────────────────────────────────────

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> Div {
        let tabs = self.tabs.iter().enumerate().map(|(index, tab)| {
            let active = index == self.active;
            let (bg, color) = if active {
                (theme::base(), theme::text())
            } else {
                (theme::crust(), theme::muted())
            };
            div()
                .id(("tab", tab.id))
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .h_full()
                .pl_3()
                .pr_1()
                .max_w(px(260.0))
                .bg(bg)
                .border_r_1()
                .border_t_2()
                .border_color(if active { theme::accent() } else { theme::crust() })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.active = index;
                    this.focus_active_tab(window, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(theme::ui_text_size())
                        .font_weight(if active { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                        .text_color(color)
                        .child(if self.tab_is_modified(index, cx) {
                            format!("● {}", tab.title())
                        } else {
                            tab.title()
                        }),
                )
                .child(
                    div()
                        .id(("close-tab", tab.id))
                        .px_1()
                        .rounded(px(3.0))
                        .text_size(theme::ui_text_size())
                        .text_color(theme::muted())
                        .hover(|style| style.bg(theme::surface_bright()).text_color(theme::text()))
                        .tooltip(|window, cx| {
                            Tooltip::new("Close tab").action(&CloseTab, None).build(window, cx)
                        })
                        .child("×")
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.focus);
                            this.request_close_tab(index, window, cx);
                        })),
                )
        });
        div()
            .flex()
            .flex_row()
            .flex_none()
            .h(px(30.0))
            .bg(theme::crust())
            .border_b_1()
            .border_color(theme::surface())
            .children(tabs)
            .child(
                div()
                    .id("new-tab")
                    .flex()
                    .items_center()
                    .px_3()
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme::text()))
                    .tooltip(|window, cx| {
                        Tooltip::new("New tab").action(&NewTab, None).build(window, cx)
                    })
                    .child(Icon::new(IconName::Plus).with_size(px(14.0)))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        window.focus(&this.focus);
                        this.add_tab(TabKind::Compare, cx);
                    })),
            )
    }

    /// The toolbar: what to compare, what to do, and how to show it.
    fn render_toolbar(&self) -> Div {
        let can_compare = ButtonState::enabled(
            self.active_tab().is_some_and(|tab| tab.is_ready() || tab.kind == TabKind::Text),
        );
        let has_tab = ButtonState::enabled(!self.tabs.is_empty());
        let has_text = ButtonState::enabled(self.text_view(self.active).is_some());
        let group = || div().flex().flex_row().flex_none().items_center().gap(px(6.0));
        let open = |content: Content, action: Box<dyn gpui::Action>, hint: &'static str| {
            button::action_button(content, action, ButtonState::Enabled, hint)
        };

        let inputs = group()
            .child(open(
                Content::Icon(IconName::SquareTerminal),
                Box::new(NewTextCompare),
                "New text compare",
            ))
            .child(button::separator())
            .child(open(
                Content::IconBadge(IconName::File, "L"),
                Box::new(OpenLeft),
                "Open left file",
            ))
            .child(open(
                Content::IconBadge(IconName::File, "R"),
                Box::new(OpenRight),
                "Open right file",
            ))
            .child(open(
                Content::IconBadge(IconName::FolderOpen, "L"),
                Box::new(OpenFolderLeft),
                "Open left folder",
            ))
            .child(open(
                Content::IconBadge(IconName::FolderOpen, "R"),
                Box::new(OpenFolderRight),
                "Open right folder",
            ))
            .child(button::separator())
            .child(open(
                Content::AppIcon(app_assets::GIT_ICON),
                Box::new(OpenGitHistory),
                "Open a file's git history",
            ));
        let actions = group()
            .child(button::primary_action_button(
                "Compare",
                Box::new(Compare),
                can_compare,
                "Compare",
            ))
            .child(button::action_button(
                "Merge…",
                Box::new(Merge),
                ButtonState::Enabled,
                "Three-way merge",
            ))
            .child(button::segmented(vec![
                Segment::action(
                    IconName::ArrowUp,
                    Box::new(PrevDifference),
                    has_tab,
                    "Previous difference",
                ),
                Segment::action(
                    IconName::ArrowDown,
                    Box::new(NextDifference),
                    has_tab,
                    "Next difference",
                ),
            ]));
        let view_options = group()
            .child(button::segmented(vec![
                Segment::action(
                    "Whitespace",
                    Box::new(ToggleIgnoreWhitespace),
                    ButtonState::toggle(self.options.diff.ignore_whitespace),
                    "Ignore whitespace differences",
                ),
                Segment::action(
                    "Case",
                    Box::new(ToggleIgnoreCase),
                    ButtonState::toggle(self.options.diff.ignore_case),
                    "Ignore case differences",
                ),
                Segment::action(
                    "Wrap",
                    Box::new(ToggleWordWrap),
                    ButtonState::toggle(self.wrap),
                    "Wrap long lines",
                ),
            ]))
            .child(button::action_button(
                "{ } Beautify",
                Box::new(Beautify),
                has_text,
                "Reformat JSON, dicts, and \\n escapes on both sides",
            ));

        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(16.0))
            .px_3()
            .h(px(36.0))
            .bg(theme::mantle())
            .border_b_1()
            .border_color(theme::surface())
            .child(inputs)
            .child(div().flex_1())
            .child(actions)
            .child(div().flex_1())
            .child(view_options)
    }

    fn render_path_bar(&self, tab: &Tab, cx: &mut Context<Self>) -> Div {
        let slots = tab.sides().iter().map(|&side| {
            let path = tab.path(side);
            let (label, color) = match path {
                Some(path) => (format::path_label(path), theme::text()),
                None => (format!("Drop a {} file or folder here", side.label()), theme::muted()),
            };
            let pickers = match side {
                Side::Base => vec![(PickKind::File, "File…")],
                Side::Left | Side::Right => {
                    vec![(PickKind::File, "File…"), (PickKind::Folder, "Folder…")]
                }
            };
            div()
                .id(SharedString::from(format!("path-{}", side.label())))
                .flex()
                .flex_row()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_1()
                .px_2()
                .h_full()
                .border_r_1()
                .border_color(theme::surface())
                .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(theme::accent_translucent()))
                .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                    this.drop_paths(paths.paths(), Some(side), cx);
                }))
                .child(
                    div()
                        .flex_none()
                        .text_size(theme::meta_text_size())
                        .text_color(theme::muted())
                        .child(format!("{}:", capitalize(side.label()))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(theme::ui_text_size())
                        .text_color(color)
                        .child(label),
                )
                .children(pickers.into_iter().map(|(kind, label)| {
                    button::button(
                        SharedString::from(format!("pick-{}-{label}", side.label())),
                        label,
                        ButtonState::Enabled,
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.pick_path(side, kind, cx)
                        }),
                    )
                }))
        });
        div()
            .flex()
            .flex_row()
            .flex_none()
            .h(px(36.0))
            .bg(theme::crust())
            .border_b_1()
            .border_color(theme::surface())
            .children(slots)
    }

    fn render_content(&self) -> gpui::AnyElement {
        let Some(tab) = self.active_tab() else {
            return render_welcome().into_any_element();
        };
        match &tab.content {
            TabContent::Empty => render_empty_tab(tab).into_any_element(),
            TabContent::Loading { progress } => {
                render_loading(progress.as_deref()).into_any_element()
            }
            TabContent::Text(view) => view.clone().into_any_element(),
            TabContent::Binary(view) => view.clone().into_any_element(),
            TabContent::Folder(view) => view.clone().into_any_element(),
            TabContent::Merge(view) => view.clone().into_any_element(),
            TabContent::Error(message) => {
                centered_message(message.clone(), theme::red()).into_any_element()
            }
        }
    }

    fn status_summary(&self, cx: &App) -> (String, Hsla) {
        let Some(tab) = self.active_tab() else {
            return ("No comparison open".to_owned(), theme::muted());
        };
        match &tab.content {
            TabContent::Empty => ("Choose items to compare".to_owned(), theme::muted()),
            TabContent::Loading { .. } => ("Comparing…".to_owned(), theme::yellow()),
            TabContent::Text(view) => (view.read(cx).summary(), theme::subtext()),
            TabContent::Binary(view) => (view.read(cx).summary(), theme::subtext()),
            TabContent::Folder(view) => (view.read(cx).summary(), theme::subtext()),
            TabContent::Merge(view) => (view.read(cx).summary(), theme::subtext()),
            TabContent::Error(message) => (message.to_string(), theme::red()),
        }
    }

    fn render_status_bar(&self, cx: &App) -> Div {
        let (summary, color) = self.status_summary(cx);
        let tab_position = if self.tabs.is_empty() {
            String::new()
        } else {
            format!("Tab {}/{}", self.active + 1, self.tabs.len())
        };
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .justify_between()
            .gap_4()
            .h(px(24.0))
            .px_3()
            .bg(theme::mantle())
            .border_t_1()
            .border_color(theme::surface())
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(theme::summary_text_size())
                    .text_color(if color == theme::muted() { color } else { theme::subtext() })
                    .child(summary),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_4()
                    .text_size(theme::meta_text_size())
                    .text_color(theme::gutter())
                    .child("⌘O/⇧⌘O open · ⌘↩ compare · ⌥↑/⌥↓ diffs · ⌘[ ⌘] tabs · ⌘W close")
                    .child(tab_position),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .active_tab()
            .map_or_else(|| "compare-rs".to_owned(), |tab| format!("{} — compare-rs", tab.title()));
        window.set_window_title(&title);
        self.save_session_if_changed(cx);
        // If the focused element went away (e.g. its tab closed), take focus
        // back so keyboard shortcuts keep working. Newly created elements
        // (an editor or find field) may not be drawn yet, so only an empty
        // focus counts as lost.
        if window.focused(cx).is_none() {
            window.focus(&self.focus);
        }

        // Once a comparison is shown, its headers name the files instead.
        let path_bar = self
            .active_tab()
            .filter(|tab| !tab.sides().is_empty())
            .filter(|tab| {
                matches!(
                    tab.content,
                    TabContent::Empty | TabContent::Loading { .. } | TabContent::Error(_)
                )
            })
            .map(|tab| self.render_path_bar(tab, cx));
        div()
            .id("workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_new_text_compare))
            .on_action(cx.listener(Self::on_beautify))
            .on_action(cx.listener(Self::on_open_git_history))
            .on_action(cx.listener(Self::on_toggle_word_wrap))
            .on_action(cx.listener(Self::on_open_left))
            .on_action(cx.listener(Self::on_open_right))
            .on_action(cx.listener(Self::on_open_folder_left))
            .on_action(cx.listener(Self::on_open_folder_right))
            .on_action(cx.listener(Self::on_compare))
            .on_action(cx.listener(Self::on_compare_folders))
            .on_action(cx.listener(Self::on_merge))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(|this, _: &Quit, window, cx| this.request_quit(window, cx)))
            .on_action(cx.listener(Self::on_save_file))
            .on_action(cx.listener(Self::on_save_all))
            .on_action(cx.listener(Self::on_revert_file))
            .on_action(cx.listener(Self::on_open_find))
            .on_action(cx.listener(Self::on_open_replace))
            .on_action(cx.listener(Self::on_find_next))
            .on_action(cx.listener(Self::on_find_prev))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_next_difference))
            .on_action(cx.listener(Self::on_prev_difference))
            .on_action(cx.listener(Self::on_toggle_ignore_whitespace))
            .on_action(cx.listener(Self::on_toggle_ignore_case))
            .on_action(cx.listener(Self::on_copy_paths))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.drop_paths(paths.paths(), None, cx)
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::base())
            .text_color(theme::text())
            .child(self.render_tab_bar(cx))
            .child(self.render_toolbar())
            .children(path_bar)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.render_content()),
            )
            .child(self.render_status_bar(cx))
    }
}

// ─── Background jobs ────────────────────────────────────────────────────────

fn compare_folder_targets(left: &Path, right: &Path, progress: &FolderProgress) -> CompareOutcome {
    let open = |path: &Path| {
        CompareTarget::open_folder(path)
            .map_err(|e| format!("Could not open {}: {e}", path.display()))
    };
    let targets = match (open(left), open(right)) {
        (Ok(l), Ok(r)) => vec![l, r],
        (Err(e), _) | (_, Err(e)) => return CompareOutcome::Failed(e),
    };
    match compare_core::compare_folders(targets[0].folder(), targets[1].folder(), progress) {
        Ok(comparison) => CompareOutcome::Folders { comparison, targets },
        Err(e) => CompareOutcome::Failed(format!("Could not compare folders: {e}")),
    }
}

fn compare_file_pair(left: &Path, right: &Path, options: &CompareOptions) -> CompareOutcome {
    match compare_core::compare_files(left, right, options) {
        Ok(comparison) => {
            let syntax = match &comparison.content {
                ComparisonContent::Text(diff) => {
                    [DocumentSyntax::new(left, &diff.left), DocumentSyntax::new(right, &diff.right)]
                }
                ComparisonContent::Binary(_) => Default::default(),
            };
            CompareOutcome::Files { comparison, syntax }
        }
        Err(e) => CompareOutcome::Failed(format!("Could not compare files: {e}")),
    }
}

/// Opens the git history of `path` and loads the first pair of versions.
fn git_history(path: &Path, options: &DiffOptions) -> CompareOutcome {
    let file = match GitFile::open(path) {
        Ok(file) => Arc::new(file),
        Err(error) => {
            return CompareOutcome::Failed(format!("Could not read git history: {error}"));
        }
    };
    let (older, newer) = file.initial_pair();
    let read = |index: usize| {
        file.read(index).map_err(|error| format!("Could not read a git version: {error}"))
    };
    let (left_bytes, right_bytes) = match (read(older), read(newer)) {
        (Ok(left), Ok(right)) => (left, right),
        (Err(error), _) | (_, Err(error)) => return CompareOutcome::Failed(error),
    };
    let infos = [
        file.version_info(older, left_bytes.len() as u64),
        file.version_info(newer, right_bytes.len() as u64),
    ];
    let identical = left_bytes == right_bytes;
    let diff = TextDiff::new(
        TextDocument::decode(&left_bytes),
        TextDocument::decode(&right_bytes),
        options,
    );
    let syntax = [
        DocumentSyntax::new(file.path(), &diff.left),
        DocumentSyntax::new(file.path(), &diff.right),
    ];
    CompareOutcome::History { file, shown: [older, newer], infos, identical, diff, syntax }
}

fn merge_files(left: &Path, base: &Path, right: &Path, options: &DiffOptions) -> CompareOutcome {
    let read = |path: &Path| {
        std::fs::read(path)
            .map(|bytes| TextDocument::decode(&bytes))
            .map_err(|e| format!("Could not read {}: {e}", path.display()))
    };
    match (read(left), read(base), read(right)) {
        (Ok(l), Ok(b), Ok(r)) => CompareOutcome::Merge {
            infos: [
                FileInfo::from_path(left),
                FileInfo::from_path(base),
                FileInfo::from_path(right),
            ],
            syntax: [
                DocumentSyntax::new(left, &l),
                DocumentSyntax::new(base, &b),
                DocumentSyntax::new(right, &r),
            ],
            document: MergeDocument::new(l, b, r, options),
        },
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => CompareOutcome::Failed(e),
    }
}

// ─── Static content ─────────────────────────────────────────────────────────

fn prompt_options(side: Side, kind: PickKind) -> PathPromptOptions {
    let noun = if kind == PickKind::File { "file" } else { "folder" };
    PathPromptOptions {
        files: kind == PickKind::File,
        directories: kind == PickKind::Folder,
        multiple: false,
        prompt: Some(SharedString::from(format!("Select {} {noun}", side.label()))),
    }
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

fn shortcut_hint(keys: &'static str, label: &'static str) -> Div {
    div()
        .flex()
        .gap_2()
        .px_3()
        .py_1()
        .rounded(px(4.0))
        .bg(theme::surface())
        .text_size(theme::ui_text_size())
        .child(div().text_color(theme::accent()).child(keys))
        .child(div().text_color(theme::subtext()).child(label))
}

fn render_welcome() -> Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .gap_4()
        .child(div().text_size(px(32.0)).font_weight(gpui::FontWeight::BOLD).child("compare-rs"))
        .child(
            div()
                .text_size(px(14.0))
                .text_color(theme::muted())
                .child("Drop two files or folders here, or open them with the toolbar"),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .justify_center()
                .gap_2()
                .child(shortcut_hint("⌘O", "left file"))
                .child(shortcut_hint("⇧⌘O", "right file"))
                .child(shortcut_hint("⇧⌘E", "left folder"))
                .child(shortcut_hint("⇧⌘R", "right folder"))
                .child(shortcut_hint("⇧⌘M", "three-way merge"))
                .child(shortcut_hint("⌥⌘G", "git history"))
                .child(shortcut_hint("⌘T", "new tab")),
        )
}

fn render_empty_tab(tab: &Tab) -> Div {
    let missing: Vec<&str> =
        tab.sides().iter().filter(|&&s| tab.path(s).is_none()).map(|s| s.label()).collect();
    div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(div().text_size(px(16.0)).text_color(theme::subtext()).child(format!(
            "Choose the {} {} to compare",
            missing.join(" and "),
            if missing.len() == 1 { "item" } else { "items" },
        )))
        .child(div().text_size(px(12.0)).text_color(theme::muted()).child(
            "Drop files or folders onto the path bar above, or use the File…/Folder… buttons",
        ))
}

fn render_loading(progress: Option<&FolderProgress>) -> Div {
    let detail = progress.map_or_else(String::new, |progress| {
        format!(
            "{} entries scanned · {} compared",
            progress.entries_scanned(),
            format::format_size(progress.bytes_compared())
        )
    });
    div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(div().text_size(px(16.0)).text_color(theme::yellow()).child("Comparing…"))
        .child(div().text_size(px(12.0)).text_color(theme::muted()).child(detail))
}

fn centered_message(message: SharedString, color: Hsla) -> Div {
    div()
        .flex()
        .size_full()
        .items_center()
        .justify_center()
        .px_8()
        .text_size(px(14.0))
        .text_color(color)
        .child(message)
}
