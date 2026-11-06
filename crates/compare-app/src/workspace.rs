//! The main window: tabs, toolbar, path bar, content, and status bar.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext, ClickEvent, ClipboardItem, Context, Div, ExternalPaths, FocusHandle,
    Focusable, Hsla, InteractiveElement, IntoElement, ParentElement, PathPromptOptions, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
};

use compare_core::{
    CompareOptions, CompareTarget, ComparisonContent, DiffOptions, FileComparison, FileInfo,
    FolderComparison, FolderProgress, MergeDocument, TargetKind, TextDocument,
};

use crate::actions::{
    CloseTab, Compare, CompareFolders, CopyPaths, Merge, NewTab, NewTextCompare, NextDifference,
    NextTab, OpenFolderLeft, OpenFolderRight, OpenLeft, OpenRight, PrevDifference, PrevTab,
    ToggleBeautify, ToggleIgnoreCase, ToggleIgnoreWhitespace, ToggleWordWrap,
};
use crate::button::{self, ButtonState};
use crate::crash_log;
use crate::document_syntax::DocumentSyntax;
use crate::folder_diff_view::{FolderDiffView, OpenFilePair};
use crate::format;
use crate::hex_diff_view::HexDiffView;
use crate::merge_view::MergeView;
use crate::paste_compare_view::PasteCompareView;
use crate::session_state::{self, SavedBounds, SavedTab, SessionState};
use crate::tab::{Side, Tab, TabContent, TabKind};
use crate::text_diff_view::TextDiffView;
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
        self.options.beautify = session.beautify;
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
            .filter(|tab| tab.kind != TabKind::Text)
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
            beautify: self.options.beautify,
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

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    /// Returns the active tab's id, creating a comparison tab if there is none.
    fn active_or_new_tab(&mut self, cx: &mut Context<Self>) -> u64 {
        match self.active_tab() {
            // Files chosen while a pasted-text tab is active open a new tab.
            Some(tab) if tab.kind != TabKind::Text => tab.id,
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
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
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
        let (Some(left), Some(right)) = (tab.left.clone(), tab.right.clone()) else {
            return;
        };
        let base = tab.base.clone();

        let mut progress: Option<Arc<FolderProgress>> = None;
        let job: Box<dyn FnOnce() -> CompareOutcome + Send> = match (kind, base) {
            (TabKind::Merge, Some(base)) => {
                Box::new(move || merge_files(&left, &base, &right, &options.diff))
            }
            (TabKind::Merge, None) | (TabKind::Text, _) => return,
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
                        TabContent::Text(cx.new(|cx| {
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
                        }))
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
                let subscription = cx.subscribe(&view, |this, _, event: &OpenFilePair, cx| {
                    this.open_file_pair(event.left.clone(), event.right.clone(), cx);
                });
                let tab = &mut self.tabs[index];
                tab.targets = targets;
                tab.subscription = Some(subscription);
                TabContent::Folder(view)
            }
            CompareOutcome::Merge { infos, syntax, document } => {
                TabContent::Merge(cx.new(|cx| MergeView::new(infos, syntax, document, cx)))
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
    fn open_file_pair(&mut self, left: PathBuf, right: PathBuf, cx: &mut Context<Self>) {
        let id = self.add_tab(TabKind::Compare, cx);
        self.set_tab_path(id, Side::Left, left, cx);
        self.set_tab_path(id, Side::Right, right, cx);
    }

    /// Opens paths given on the command line: two paths are compared, three
    /// are merged as `LEFT BASE RIGHT`.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
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
        if let Some(TabContent::Pasted(view)) = self.active_tab().map(|tab| &tab.content) {
            view.update(cx, |view, cx| view.compare(cx));
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
        self.close_tab(self.active, cx);
    }

    fn on_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
            cx.notify();
        }
    }

    fn on_prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        if !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            cx.notify();
        }
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
            TabContent::Pasted(view) => view.update(cx, |view, cx| {
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
        let options = self.options;
        let view = cx.new(|cx| PasteCompareView::new(options, window, cx));
        let id = self.add_tab(TabKind::Text, cx);
        if let Some(index) = self.tab_index(id) {
            self.tabs[index].content = TabContent::Pasted(view);
        }
        cx.notify();
    }

    fn on_toggle_beautify(&mut self, _: &ToggleBeautify, _: &mut Window, cx: &mut Context<Self>) {
        self.options.beautify = !self.options.beautify;
        self.recompare_text_tabs(true, cx);
    }

    fn on_toggle_ignore_whitespace(
        &mut self,
        _: &ToggleIgnoreWhitespace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.options.diff.ignore_whitespace = !self.options.diff.ignore_whitespace;
        self.recompare_text_tabs(false, cx);
    }

    fn on_toggle_ignore_case(
        &mut self,
        _: &ToggleIgnoreCase,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.options.diff.ignore_case = !self.options.diff.ignore_case;
        self.recompare_text_tabs(false, cx);
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
                TabContent::Pasted(view) => {
                    view.update(cx, |view, cx| view.set_wrap(wrap, window, cx));
                }
                _ => {}
            }
        }
        cx.notify();
    }

    /// Re-runs every comparison whose result depends on the diff options.
    /// Text comparisons are re-diffed in place, keeping any edits, unless
    /// Beautify changed, which needs the files read again.
    fn recompare_text_tabs(&mut self, beautify_changed: bool, cx: &mut Context<Self>) {
        let options = self.options;
        for index in 0..self.tabs.len() {
            match &self.tabs[index].content {
                TabContent::Text(view) if !beautify_changed => {
                    view.update(cx, |view, cx| view.set_options(options.diff, cx));
                }
                TabContent::Text(_) | TabContent::Merge(_) => self.start_compare(index, cx),
                TabContent::Pasted(view) => {
                    view.update(cx, |view, cx| view.set_options(options, cx));
                }
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
                    window.focus(&this.focus);
                    this.active = index;
                    cx.notify();
                }))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(color)
                        .child(tab.title()),
                )
                .child(
                    div()
                        .id(("close-tab", tab.id))
                        .px_1()
                        .rounded(px(3.0))
                        .text_size(px(11.0))
                        .text_color(theme::muted())
                        .hover(|style| style.bg(theme::surface_bright()).text_color(theme::text()))
                        .child("×")
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.focus);
                            this.close_tab(index, cx);
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
                    .child("+")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        window.focus(&this.focus);
                        this.add_tab(TabKind::Compare, cx);
                    })),
            )
    }

    fn render_toolbar(&self) -> Div {
        let can_compare =
            if self.active_tab().is_some_and(|tab| tab.is_ready() || tab.kind == TabKind::Text) {
                ButtonState::Enabled
            } else {
                ButtonState::Disabled
            };
        let has_tab =
            if self.tabs.is_empty() { ButtonState::Disabled } else { ButtonState::Enabled };
        let toggle = |on: bool| if on { ButtonState::Active } else { ButtonState::Enabled };
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_1()
            .px_2()
            .h(px(34.0))
            .bg(theme::mantle())
            .border_b_1()
            .border_color(theme::surface())
            .child(button::action_button(
                "📝 New Text",
                Box::new(NewTextCompare),
                ButtonState::Enabled,
            ))
            .child(button::separator())
            .child(button::action_button("📄 Left File", Box::new(OpenLeft), ButtonState::Enabled))
            .child(button::action_button(
                "📄 Right File",
                Box::new(OpenRight),
                ButtonState::Enabled,
            ))
            .child(button::separator())
            .child(button::action_button(
                "📁 Left Folder",
                Box::new(OpenFolderLeft),
                ButtonState::Enabled,
            ))
            .child(button::action_button(
                "📁 Right Folder",
                Box::new(OpenFolderRight),
                ButtonState::Enabled,
            ))
            .child(button::separator())
            .child(button::action_button("▶ Compare", Box::new(Compare), can_compare))
            .child(button::action_button("🔀 Merge…", Box::new(Merge), ButtonState::Enabled))
            .child(button::separator())
            .child(button::action_button("▲ Prev Diff", Box::new(PrevDifference), has_tab))
            .child(button::action_button("▼ Next Diff", Box::new(NextDifference), has_tab))
            .child(button::separator())
            .child(button::action_button(
                "Ignore Whitespace",
                Box::new(ToggleIgnoreWhitespace),
                toggle(self.options.diff.ignore_whitespace),
            ))
            .child(button::action_button(
                "Ignore Case",
                Box::new(ToggleIgnoreCase),
                toggle(self.options.diff.ignore_case),
            ))
            .child(button::action_button(
                "{ } Beautify",
                Box::new(ToggleBeautify),
                toggle(self.options.beautify),
            ))
            .child(button::action_button("↩ Wrap", Box::new(ToggleWordWrap), toggle(self.wrap)))
            .child(div().flex_1())
            .child(button::action_button("＋ New Tab", Box::new(NewTab), ButtonState::Enabled))
            .child(button::action_button("✕ Close Tab", Box::new(CloseTab), has_tab))
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
                        .text_size(px(10.0))
                        .text_color(theme::muted())
                        .child(format!("{}:", capitalize(side.label()))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.0))
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
            .h(px(28.0))
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
            TabContent::Pasted(view) => view.clone().into_any_element(),
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
            TabContent::Pasted(view) => (view.read(cx).summary(cx), theme::subtext()),
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
            .text_size(px(10.0))
            .child(div().min_w_0().truncate().text_color(color).child(summary))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_4()
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
        // back so keyboard shortcuts keep working.
        if !self.focus.contains_focused(window, cx) {
            window.focus(&self.focus);
        }

        let path_bar = self
            .active_tab()
            .filter(|tab| !tab.sides().is_empty())
            .map(|tab| self.render_path_bar(tab, cx));
        div()
            .id("workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_new_text_compare))
            .on_action(cx.listener(Self::on_toggle_beautify))
            .on_action(cx.listener(Self::on_toggle_word_wrap))
            .on_action(cx.listener(Self::on_open_left))
            .on_action(cx.listener(Self::on_open_right))
            .on_action(cx.listener(Self::on_open_folder_left))
            .on_action(cx.listener(Self::on_open_folder_right))
            .on_action(cx.listener(Self::on_compare))
            .on_action(cx.listener(Self::on_compare_folders))
            .on_action(cx.listener(Self::on_merge))
            .on_action(cx.listener(Self::on_close_tab))
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
        .text_size(px(11.0))
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
