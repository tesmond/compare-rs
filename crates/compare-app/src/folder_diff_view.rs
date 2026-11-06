//! Side-by-side tree view of a folder comparison.

use std::cmp::Ordering;
use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::{
    App, ClickEvent, Context, Div, EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Render, ScrollStrategy, SharedString,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, div, px, rgb,
    uniform_list,
};
use gpui_component::menu::ContextMenuExt;
use gpui_component::{Icon, IconName, Sizable};

use compare_core::{EntryMeta, FolderComparison, FolderNode, FolderStatus};

use crate::actions::{
    Collapse, CopyLeftPath, CopyRightPath, Expand, FOLDER_VIEW_CONTEXT, OpenSelected, RevealLeft,
    RevealRight, SelectNext, SelectPrev,
};
use crate::button::{self, ButtonState};
use crate::file_header;
use crate::format;
use crate::path_actions;
use crate::theme;

const INDENT_WIDTH: f32 = 14.0;
const SIZE_COLUMN_WIDTH: f32 = 72.0;
const TIME_COLUMN_WIDTH: f32 = 112.0;

/// Emitted when the user asks to compare a pair of files from the tree.
pub struct OpenFilePair {
    pub left: PathBuf,
    pub right: PathBuf,
}

/// What the context menu acts on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MenuTarget {
    is_dir: bool,
    is_file_pair: bool,
    has_left: bool,
    has_right: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SortColumn {
    Name,
    Size,
    Modified,
}

/// Which side of the tree a column header belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

/// Shows the merged left/right folder tree with each entry's status.
pub struct FolderDiffView {
    focus: FocusHandle,
    comparison: FolderComparison,
    left_label: String,
    right_label: String,
    /// Expansion state for each node in `comparison.nodes`.
    expanded: Vec<bool>,
    /// Node indices currently shown, in display order.
    visible: Vec<usize>,
    selected: Option<usize>,
    sort: SortColumn,
    ascending: bool,
    differences_only: bool,
    scroll: UniformListScrollHandle,
}

impl EventEmitter<OpenFilePair> for FolderDiffView {}

impl Focusable for FolderDiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FolderDiffView {
    /// Creates the view. Directories containing differences start expanded.
    pub fn new(
        comparison: FolderComparison,
        left_label: String,
        right_label: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let expanded =
            comparison.nodes.iter().map(|node| node.status != FolderStatus::Identical).collect();
        let mut view = Self {
            focus: cx.focus_handle(),
            comparison,
            left_label,
            right_label,
            expanded,
            visible: Vec::new(),
            selected: None,
            sort: SortColumn::Name,
            ascending: true,
            differences_only: false,
            scroll: UniformListScrollHandle::new(),
        };
        view.rebuild_visible();
        view
    }

    /// Re-checks the file pair that includes `path` after it was saved.
    pub fn refresh_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.comparison.refresh_file(path) {
            self.rebuild_visible();
            cx.notify();
        }
    }

    /// One-line description for the status bar.
    pub fn summary(&self) -> String {
        let stats = self.comparison.stats;
        format!(
            "Files — identical: {} · different: {} · left only: {} · right only: {}{}",
            stats.identical,
            stats.different,
            stats.left_only,
            stats.right_only,
            if stats.errors > 0 { format!(" · errors: {}", stats.errors) } else { String::new() },
        )
    }

    /// Paths of the selected entry on each side, for copying.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        let Some(node) = self.selected.map(|index| &self.comparison.nodes[index]) else {
            return vec![self.comparison.left_root.clone(), self.comparison.right_root.clone()];
        };
        let mut paths = Vec::new();
        if node.left.is_some() {
            paths.push(self.comparison.left_path(node));
        }
        if node.right.is_some() {
            paths.push(self.comparison.right_path(node));
        }
        paths
    }

    /// Selects the next entry that is not identical.
    pub fn next_difference(&mut self, cx: &mut Context<Self>) {
        self.select_difference(true, cx);
    }

    /// Selects the previous entry that is not identical.
    pub fn prev_difference(&mut self, cx: &mut Context<Self>) {
        self.select_difference(false, cx);
    }

    fn select_difference(&mut self, forward: bool, cx: &mut Context<Self>) {
        let position = self.selected_position();
        let is_difference = |&&node: &&usize| {
            let node = &self.comparison.nodes[node];
            node.status != FolderStatus::Identical && !node.is_dir()
        };
        let found = if forward {
            let start = position.map_or(0, |p| p + 1);
            self.visible[start.min(self.visible.len())..].iter().find(is_difference).copied()
        } else {
            let end = position.unwrap_or(self.visible.len());
            self.visible[..end].iter().rev().find(is_difference).copied()
        };
        if let Some(node) = found {
            self.select(node, cx);
        }
    }

    fn selected_position(&self) -> Option<usize> {
        let selected = self.selected?;
        self.visible.iter().position(|&node| node == selected)
    }

    fn select(&mut self, node: usize, cx: &mut Context<Self>) {
        self.selected = Some(node);
        if let Some(position) = self.selected_position() {
            self.scroll.scroll_to_item(position, ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn rebuild_visible(&mut self) {
        let mut visible = Vec::with_capacity(self.visible.len().max(self.comparison.roots.len()));
        let roots = self.sorted(&self.comparison.roots);
        self.push_visible(&roots, &mut visible);
        self.visible = visible;
    }

    fn push_visible(&self, nodes: &[usize], visible: &mut Vec<usize>) {
        for &index in nodes {
            let node = &self.comparison.nodes[index];
            if self.differences_only && node.status == FolderStatus::Identical {
                continue;
            }
            visible.push(index);
            if node.is_dir() && self.expanded[index] {
                let children = self.sorted(&node.children);
                self.push_visible(&children, visible);
            }
        }
    }

    fn sorted(&self, nodes: &[usize]) -> Vec<usize> {
        let mut sorted = nodes.to_vec();
        let nodes = &self.comparison.nodes;
        sorted.sort_by(|&a, &b| {
            let (a, b) = (&nodes[a], &nodes[b]);
            // Folders always stay above files.
            let by_kind = b.is_dir().cmp(&a.is_dir());
            let by_column = match self.sort {
                SortColumn::Name => Ordering::Equal,
                SortColumn::Size => largest_size(a).cmp(&largest_size(b)),
                SortColumn::Modified => latest_time(a).cmp(&latest_time(b)),
            };
            let by_column = if self.ascending { by_column } else { by_column.reverse() };
            let by_name = a.name.to_lowercase().cmp(&b.name.to_lowercase());
            let by_name = if self.sort == SortColumn::Name && !self.ascending {
                by_name.reverse()
            } else {
                by_name
            };
            by_kind.then(by_column).then(by_name)
        });
        sorted
    }

    fn set_sort(&mut self, column: SortColumn, cx: &mut Context<Self>) {
        if self.sort == column {
            self.ascending = !self.ascending;
        } else {
            self.sort = column;
            self.ascending = true;
        }
        self.rebuild_visible();
        cx.notify();
    }

    fn toggle_differences_only(&mut self, cx: &mut Context<Self>) {
        self.differences_only = !self.differences_only;
        self.rebuild_visible();
        if self.selected_position().is_none() {
            self.selected = None;
        }
        cx.notify();
    }

    fn set_expanded(&mut self, node: usize, expanded: bool, cx: &mut Context<Self>) {
        if self.comparison.nodes[node].is_dir() && self.expanded[node] != expanded {
            self.expanded[node] = expanded;
            self.rebuild_visible();
            cx.notify();
        }
    }

    /// Opens a file pair or toggles a directory.
    fn activate(&mut self, node_index: usize, cx: &mut Context<Self>) {
        let node = &self.comparison.nodes[node_index];
        if node.is_dir() {
            let expanded = !self.expanded[node_index];
            self.set_expanded(node_index, expanded, cx);
        } else if node.is_file_pair() {
            cx.emit(OpenFilePair {
                left: self.comparison.left_path(node),
                right: self.comparison.right_path(node),
            });
        }
    }

    fn on_select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let next = self.selected_position().map_or(0, |p| p + 1);
        if let Some(&node) = self.visible.get(next) {
            self.select(node, cx);
        }
    }

    fn on_select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        let prev = self.selected_position().map_or(0, |p| p.saturating_sub(1));
        if let Some(&node) = self.visible.get(prev) {
            self.select(node, cx);
        }
    }

    fn on_expand(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(node) = self.selected {
            self.set_expanded(node, true, cx);
        }
    }

    fn on_collapse(&mut self, _: &Collapse, _: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.selected else {
            return;
        };
        if self.comparison.nodes[node].is_dir() && self.expanded[node] {
            self.set_expanded(node, false, cx);
            return;
        }
        // Otherwise move the selection to the parent folder.
        let depth = self.comparison.nodes[node].depth;
        if let Some(position) = self.selected_position() {
            let parent = self.visible[..position]
                .iter()
                .rev()
                .find(|&&candidate| self.comparison.nodes[candidate].depth < depth)
                .copied();
            if let Some(parent) = parent {
                self.select(parent, cx);
            }
        }
    }

    /// Describes the selected entry for the context menu.
    fn menu_target(&self) -> Option<MenuTarget> {
        let node = &self.comparison.nodes[self.selected?];
        Some(MenuTarget {
            is_dir: node.is_dir(),
            is_file_pair: node.is_file_pair(),
            has_left: node.left.is_some(),
            has_right: node.right.is_some(),
        })
    }

    /// Path of the selected entry on one side, if it exists there.
    fn selected_side_path(&self, left: bool) -> Option<PathBuf> {
        let node = &self.comparison.nodes[self.selected?];
        match left {
            true => node.left.map(|_| self.comparison.left_path(node)),
            false => node.right.map(|_| self.comparison.right_path(node)),
        }
    }

    fn on_copy_left(&mut self, _: &CopyLeftPath, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_side_path(true) {
            path_actions::copy_path(&path, cx);
        }
    }

    fn on_copy_right(&mut self, _: &CopyRightPath, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_side_path(false) {
            path_actions::copy_path(&path, cx);
        }
    }

    fn on_reveal_left(&mut self, _: &RevealLeft, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_side_path(true) {
            path_actions::reveal(&path, cx);
        }
    }

    fn on_reveal_right(&mut self, _: &RevealRight, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_side_path(false) {
            path_actions::reveal(&path, cx);
        }
    }

    fn on_open_selected(&mut self, _: &OpenSelected, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(node) = self.selected {
            self.activate(node, cx);
        }
    }

    fn on_row_click(
        &mut self,
        node: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        self.selected = Some(node);
        if event.click_count() >= 2 {
            self.activate(node, cx);
        }
        cx.notify();
    }

    fn render_rows(
        &self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        range
            .map(|position| {
                let index = self.visible[position];
                let node = &self.comparison.nodes[index];
                let selected = self.selected == Some(index);
                let expanded = self.expanded[index];
                let background = if selected { theme::surface() } else { theme::base() };
                div()
                    .id(("folder-row", index))
                    .flex()
                    .flex_row()
                    .w_full()
                    .h(theme::row_height())
                    .bg(background)
                    .text_size(px(12.0))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus);
                            this.selected = Some(index);
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.on_row_click(index, event, window, cx);
                    }))
                    .child(div().flex_none().w(px(3.0)).h_full().bg(status_color(node.status)))
                    .child(entry_cell(node, node.left, expanded))
                    .child(div().flex_none().w(px(1.0)).h_full().bg(theme::surface()))
                    .child(entry_cell(node, node.right, expanded))
            })
            .collect()
    }

    fn column_headers(&self, side: Side, cx: &mut Context<Self>) -> Div {
        let header = |label: &'static str, column: SortColumn, cx: &mut Context<Self>| {
            let arrow = match (self.sort == column, self.ascending) {
                (false, _) => "",
                (true, true) => " ▲",
                (true, false) => " ▼",
            };
            div()
                .id(SharedString::from(format!("sort-{label}-{side:?}")))
                .cursor_pointer()
                .hover(|style| style.text_color(theme::text()))
                .child(format!("{label}{arrow}"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_sort(column, cx)))
        };
        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_w_0()
            .h(px(20.0))
            .items_center()
            .px_2()
            .bg(theme::crust())
            .text_size(theme::meta_text_size())
            .text_color(theme::muted())
            .child(div().flex_1().child(header("Name", SortColumn::Name, cx)))
            .child(div().flex_none().w(px(SIZE_COLUMN_WIDTH)).flex().justify_end().child(header(
                "Size",
                SortColumn::Size,
                cx,
            )))
            .child(div().flex_none().w(px(TIME_COLUMN_WIDTH)).pl_3().child(header(
                "Modified",
                SortColumn::Modified,
                cx,
            )))
    }
}

/// One side of a tree row.
fn entry_cell(node: &FolderNode, meta: Option<EntryMeta>, expanded: bool) -> Div {
    let cell =
        div().flex().flex_row().flex_1().min_w_0().h_full().items_center().px_2().overflow_hidden();
    let Some(meta) = meta else {
        return cell.bg(theme::filler_bg());
    };
    let is_dir = meta.kind == compare_core::EntryKind::Dir;
    let disclosure = match (is_dir, expanded) {
        (false, _) => " ",
        (true, true) => "▾",
        (true, false) => "▸",
    };
    let icon = if is_dir { IconName::Folder } else { IconName::File };
    let size = if is_dir { String::new() } else { format::format_size(meta.size) };
    cell.child(
        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_1()
            .pl(px(f32::from(node.depth) * INDENT_WIDTH))
            .child(div().flex_none().w(px(10.0)).text_color(theme::muted()).child(disclosure))
            .child(Icon::new(icon).with_size(px(14.0)).text_color(theme::subtext()))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(name_color(node.status))
                    .child(node.name.clone()),
            ),
    )
    .child(
        div()
            .flex_none()
            .w(px(SIZE_COLUMN_WIDTH))
            .flex()
            .justify_end()
            .text_size(theme::ui_text_size())
            .text_color(theme::subtext())
            .child(size),
    )
    .child(
        div()
            .flex_none()
            .w(px(TIME_COLUMN_WIDTH))
            .pl_3()
            .text_size(theme::ui_text_size())
            .text_color(theme::muted())
            .whitespace_nowrap()
            .child(format::format_time(meta.modified)),
    )
}

fn status_color(status: FolderStatus) -> Hsla {
    match status {
        FolderStatus::Identical => theme::base(),
        FolderStatus::Different => theme::red(),
        FolderStatus::LeftOnly | FolderStatus::RightOnly => orphan_color(),
        FolderStatus::Error => theme::yellow(),
    }
}

fn name_color(status: FolderStatus) -> Hsla {
    match status {
        FolderStatus::Identical => theme::text(),
        FolderStatus::Different => theme::red(),
        FolderStatus::LeftOnly | FolderStatus::RightOnly => orphan_color(),
        FolderStatus::Error => theme::yellow(),
    }
}

fn orphan_color() -> Hsla {
    rgb(0xcba6f7).into()
}

fn largest_size(node: &FolderNode) -> u64 {
    [node.left, node.right].iter().flatten().map(|meta| meta.size).max().unwrap_or(0)
}

fn latest_time(node: &FolderNode) -> Option<std::time::SystemTime> {
    [node.left, node.right].iter().flatten().filter_map(|meta| meta.modified).max()
}

impl Render for FolderDiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let filter_state =
            if self.differences_only { ButtonState::Active } else { ButtonState::Enabled };
        div()
            .key_context(FOLDER_VIEW_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_prev))
            .on_action(cx.listener(Self::on_expand))
            .on_action(cx.listener(Self::on_collapse))
            .on_action(cx.listener(Self::on_open_selected))
            .on_action(cx.listener(Self::on_copy_left))
            .on_action(cx.listener(Self::on_copy_right))
            .on_action(cx.listener(Self::on_reveal_left))
            .on_action(cx.listener(Self::on_reveal_right))
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(file_header::header_bar(self.left_label.clone(), "Left folder"))
                    .child(div().w(px(1.0)).bg(theme::surface()))
                    .child(file_header::header_bar(self.right_label.clone(), "Right folder")),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py(px(3.0))
                    .bg(theme::mantle())
                    .border_b_1()
                    .border_color(theme::surface())
                    .child(button::button(
                        "differences-only",
                        "Show differences only",
                        filter_state,
                        cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_differences_only(cx)),
                    ))
                    .child(
                        div().text_size(theme::meta_text_size()).text_color(theme::muted()).child(
                            "Double-click or press Enter to compare files · ←/→ collapse/expand",
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(self.column_headers(Side::Left, cx))
                    .child(div().w(px(1.0)).bg(theme::surface()))
                    .child(self.column_headers(Side::Right, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "folder-rows",
                            self.visible.len(),
                            cx.processor(|this, range: Range<usize>, _window, cx| {
                                this.render_rows(range, cx)
                            }),
                        )
                        .track_scroll(self.scroll.clone())
                        .flex_1()
                        .bg(theme::base()),
                    )
                    .context_menu({
                        let focus = self.focus.clone();
                        let view = cx.entity().downgrade();
                        move |menu, _, cx| {
                            // Read the selection when the menu opens: the
                            // right-click that opened it may have changed it.
                            let target = view
                                .upgrade()
                                .and_then(|view| view.read(cx).menu_target())
                                .unwrap_or_default();
                            let open_label =
                                if target.is_dir { "Expand / Collapse" } else { "Compare Files" };
                            let menu = menu
                                .action_context(focus.clone())
                                .menu_with_disabled(
                                    open_label,
                                    Box::new(OpenSelected),
                                    !(target.is_dir || target.is_file_pair),
                                )
                                .separator();
                            path_actions::add_path_items(menu, target.has_left, target.has_right)
                        }
                    }),
            )
    }
}
