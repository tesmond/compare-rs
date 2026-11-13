//! Comparing two git branches: a picker for the branches, then the same
//! tree view as a folder comparison.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle,
    Window, div, px, uniform_list,
};

use compare_core::{CompareTarget, FolderComparison, GitBranches};

use crate::button::{self, ButtonState};
use crate::crash_log;
use crate::file_header;
use crate::folder_diff_view::{FolderDiffView, OpenFilePair};
use crate::format;
use crate::theme;

/// Shown on files opened from a branch comparison.
const READ_ONLY: &str = "Read-only: a git branch";

/// Branches offered for the left side when the repository has them, in
/// order of preference.
const USUAL_BASES: [&str; 3] = ["main", "master", "develop"];

/// The left and right sides, as indices.
const SIDES: [usize; 2] = [0, 1];

/// A comparison of two branches that is on screen.
struct Shown {
    view: Entity<FolderDiffView>,
    /// The branches on the left and right.
    names: [String; 2],
    /// Temporary folders that files are copied into when they are opened.
    targets: Vec<Arc<CompareTarget>>,
    _subscription: Subscription,
}

/// Lets the user pick two branches of a repository, then shows how their
/// files differ.
pub struct BranchCompareView {
    focus: FocusHandle,
    branches: GitBranches,
    /// The branch picked for each side, as an index into the branch names.
    chosen: [Option<usize>; 2],
    shown: Option<Shown>,
    /// True while the branch lists are shown rather than the comparison.
    picking: bool,
    loading: Option<Task<()>>,
    error: Option<SharedString>,
    scroll: [UniformListScrollHandle; 2],
}

impl EventEmitter<OpenFilePair> for BranchCompareView {}

impl Focusable for BranchCompareView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BranchCompareView {
    /// Creates the view on its picker, with the checked-out branch on the
    /// right and the repository's main branch (if it has one) on the left.
    pub fn new(branches: GitBranches, cx: &mut Context<Self>) -> Self {
        let position = |name: &str| branches.names.iter().position(|branch| branch == name);
        let right = branches.current.as_deref().and_then(position);
        let left =
            USUAL_BASES.iter().find_map(|name| position(name)).filter(|&base| Some(base) != right);
        Self {
            focus: cx.focus_handle(),
            branches,
            chosen: [left, right],
            shown: None,
            picking: true,
            loading: None,
            error: None,
            scroll: [UniformListScrollHandle::new(), UniformListScrollHandle::new()],
        }
    }

    /// The temporary folders holding files opened from the comparison. A
    /// tab showing one of those files keeps them, so the files stay on disk
    /// while it is open.
    pub fn targets(&self) -> Vec<Arc<CompareTarget>> {
        self.shown.as_ref().map(|shown| shown.targets.clone()).unwrap_or_default()
    }

    /// One-line description for the status bar.
    pub fn summary(&self, cx: &App) -> String {
        match &self.shown {
            Some(shown) if !self.picking => {
                let [left, right] = &shown.names;
                format!("{left} ↔ {right} · {}", shown.view.read(cx).summary())
            }
            _ => "Choose two branches to compare".to_owned(),
        }
    }

    /// Returns true if a comparison is shown and the branches differ.
    pub fn has_differences(&self, cx: &App) -> bool {
        !self.picking && self.shown.as_ref().is_some_and(|s| s.view.read(cx).has_differences())
    }

    pub fn next_difference(&mut self, cx: &mut Context<Self>) {
        if let Some(shown) = self.shown.as_ref().filter(|_| !self.picking) {
            shown.view.update(cx, |view, cx| view.next_difference(cx));
        }
    }

    pub fn prev_difference(&mut self, cx: &mut Context<Self>) {
        if let Some(shown) = self.shown.as_ref().filter(|_| !self.picking) {
            shown.view.update(cx, |view, cx| view.prev_difference(cx));
        }
    }

    /// Compares the two chosen branches, on a background thread.
    pub fn compare(&mut self, cx: &mut Context<Self>) {
        let [Some(left), Some(right)] = self.chosen else {
            return;
        };
        let names = [self.branches.names[left].clone(), self.branches.names[right].clone()];
        let root = self.branches.root.clone();
        self.error = None;
        self.loading = Some(cx.spawn(async move |this, cx| {
            let job_names = names.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let target = |name: &str| {
                        CompareTarget::snapshot(root.clone(), name, READ_ONLY)
                            .map_err(|error| error.to_string())
                    };
                    let targets = [target(&job_names[0])?, target(&job_names[1])?];
                    let comparison = compare_core::compare_branches(
                        &root,
                        &job_names[0],
                        &job_names[1],
                        targets[0].folder(),
                        targets[1].folder(),
                    )
                    .map_err(|error| error.to_string())?;
                    Ok::<_, String>((comparison, targets))
                })
                .await;
            this.update(cx, |this, cx| {
                this.loading = None;
                match result {
                    Ok((comparison, targets)) => this.show(comparison, targets, names, cx),
                    Err(error) => {
                        crash_log::record_error("Comparing branches", &error);
                        this.error =
                            Some(format!("Could not compare the branches: {error}").into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn show(
        &mut self,
        comparison: FolderComparison,
        targets: [CompareTarget; 2],
        names: [String; 2],
        cx: &mut Context<Self>,
    ) {
        let repository = format::file_name(&self.branches.root);
        let label = |name: &str| format!("{name} — {repository}");
        let (left, right) = (label(&names[0]), label(&names[1]));
        let view = cx.new(|cx| FolderDiffView::new(comparison, left, right, cx).for_git_branches());
        let subscription = cx.subscribe(&view, |this, _, event: &OpenFilePair, cx| {
            this.open_files(&event.left, cx);
        });
        self.shown = Some(Shown {
            view,
            names,
            targets: targets.into_iter().map(Arc::new).collect(),
            _subscription: subscription,
        });
        self.picking = false;
    }

    /// Copies both branches' versions of a file out of git, then asks for
    /// them to be opened as a comparison. `left` is where the left version
    /// goes, inside the left temporary folder.
    fn open_files(&mut self, left: &Path, cx: &mut Context<Self>) {
        let Some(shown) = &self.shown else {
            return;
        };
        let Ok(relative) = left.strip_prefix(shown.targets[0].folder()) else {
            return;
        };
        let relative = relative.to_path_buf();
        let root = self.branches.root.clone();
        let sides = SIDES
            .map(|side| (shown.names[side].clone(), shown.targets[side].folder().to_path_buf()));
        cx.spawn(async move |this, cx| {
            let exported = cx
                .background_executor()
                .spawn(async move {
                    let [left, right] = sides.map(|(branch, folder)| {
                        compare_core::export_branch_file(&root, &branch, &relative, &folder)
                    });
                    Ok::<_, compare_core::GitError>((left?, right?))
                })
                .await;
            this.update(cx, |this, cx| match exported {
                Ok((left, right)) => cx.emit(OpenFilePair { left, right }),
                Err(error) => {
                    crash_log::record_error("Reading a file from a branch", &error.to_string());
                    this.error = Some(format!("Could not read the file from git: {error}").into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn swap(&mut self, cx: &mut Context<Self>) {
        self.chosen.swap(0, 1);
        self.compare(cx);
    }

    /// The strip above the content: what is compared, and the buttons that
    /// change it.
    fn render_bar(&self, cx: &mut Context<Self>) -> Div {
        let bar = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .px_3()
            .h(px(36.0))
            .bg(theme::mantle())
            .border_b_1()
            .border_color(theme::surface())
            .text_size(theme::ui_text_size())
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::text())
                    .child(format::file_name(&self.branches.root)),
            );
        let note = |text: SharedString| {
            div().flex_1().min_w_0().truncate().text_color(theme::subtext()).child(text)
        };
        match &self.shown {
            Some(shown) if !self.picking => {
                let [left, right] = &shown.names;
                bar.child(note(format!("{left}  ↔  {right}").into()))
                    .child(button::with_hint(
                        button::button(
                            "swap-branches",
                            "Swap",
                            ButtonState::Enabled,
                            cx.listener(|this, _: &ClickEvent, _, cx| this.swap(cx)),
                        ),
                        button::Hint::text("Swap the left and right branches"),
                    ))
                    .child(button::button(
                        "change-branches",
                        "Change Branches…",
                        ButtonState::Enabled,
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.picking = true;
                            cx.notify();
                        }),
                    ))
            }
            shown => {
                let ready = self.chosen.iter().all(Option::is_some) && self.loading.is_none();
                let compare = button::button(
                    "compare-branches",
                    "Compare Branches",
                    ButtonState::enabled(ready),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.compare(cx)),
                )
                .px_3()
                .font_weight(FontWeight::SEMIBOLD);
                let compare = if ready {
                    compare
                        .bg(theme::accent())
                        .text_color(theme::crust())
                        .hover(|style| style.bg(theme::text()).text_color(theme::crust()))
                } else {
                    compare
                };
                let cancel = shown.is_some().then(|| {
                    button::button(
                        "cancel-branches",
                        "Cancel",
                        ButtonState::Enabled,
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            // Go back to the branches that are compared.
                            if let Some(shown) = &this.shown {
                                let find = |name: &String| {
                                    this.branches.names.iter().position(|branch| branch == name)
                                };
                                this.chosen = [find(&shown.names[0]), find(&shown.names[1])];
                            }
                            this.picking = false;
                            cx.notify();
                        }),
                    )
                });
                bar.child(note("Choose a branch for each side".into()))
                    .children(cancel)
                    .child(compare)
            }
        }
    }

    /// Two lists of the repository's branches, one for each side.
    fn render_picker(&self, cx: &mut Context<Self>) -> Div {
        let column = |side: usize, cx: &mut Context<Self>| {
            let (caption, id) = match side {
                0 => ("Left branch", "branches-left"),
                _ => ("Right branch", "branches-right"),
            };
            let chosen = self.chosen[side].map(|index| self.branches.names[index].clone());
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(div().flex().flex_row().flex_none().child(file_header::header_bar(
                    chosen.unwrap_or_else(|| "No branch chosen".to_owned()),
                    caption,
                )))
                .child(
                    uniform_list(
                        id,
                        self.branches.names.len(),
                        cx.processor(move |this, range: Range<usize>, _window, cx| {
                            this.render_branch_rows(side, range, cx)
                        }),
                    )
                    .track_scroll(self.scroll[side].clone())
                    .flex_1()
                    .bg(theme::base()),
                )
        };
        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h_0()
            .child(column(0, cx))
            .child(div().flex_none().w(px(1.0)).bg(theme::surface()))
            .child(column(1, cx))
    }

    fn render_branch_rows(
        &self,
        side: usize,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<Div>> {
        range
            .map(|index| {
                let name = &self.branches.names[index];
                let selected = self.chosen[side] == Some(index);
                let current = self.branches.current.as_ref() == Some(name);
                let id = if side == 0 { "branch-left" } else { "branch-right" };
                let row = div()
                    .id((id, index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .h(theme::row_height())
                    .px_3()
                    .text_size(theme::ui_text_size())
                    .cursor_pointer()
                    .child(div().min_w_0().truncate().child(SharedString::from(name.clone())))
                    .children(current.then(|| {
                        div()
                            .flex_none()
                            .text_size(theme::meta_text_size())
                            .text_color(theme::muted())
                            .child("checked out")
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        window.focus(&this.focus);
                        this.chosen[side] = Some(index);
                        cx.notify();
                    }));
                if selected {
                    row.bg(theme::active_bg()).text_color(theme::text())
                } else {
                    row.bg(theme::base())
                        .text_color(theme::subtext())
                        .hover(|style| style.bg(theme::surface()))
                }
            })
            .collect()
    }
}

impl Render for BranchCompareView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = match &self.shown {
            _ if self.loading.is_some() => div()
                .flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_size(px(16.0))
                .text_color(theme::yellow())
                .child("Comparing branches…")
                .into_any_element(),
            Some(shown) if !self.picking => shown.view.clone().into_any_element(),
            _ => self.render_picker(cx).into_any_element(),
        };
        div()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_bar(cx))
            .children(self.error.clone().map(|error| file_header::banner(error, theme::red())))
            .child(div().flex().flex_col().flex_1().min_h_0().child(body))
    }
}
