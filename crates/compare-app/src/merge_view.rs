//! Three-way merge view: left / base / right panes plus a merged output pane.

use std::path::PathBuf;

use gpui::{
    AnyElement, AppContext, ClickEvent, Context, Div, Entity, FocusHandle, FontWeight, Hsla,
    InteractiveElement, IntoElement, ListAlignment, ListOffset, ListState, ParentElement, Pixels,
    Render, ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled, Window, div, list,
    px,
};
use gpui_component::IconName;
use gpui_component::input::{Input, InputState};

use compare_core::{FileInfo, MergeDocument, OutputLine, Resolution, TextDocument};
use compare_diff::MergeRegionKind;

use crate::actions::{NextDifference, PrevDifference};
use crate::button::{self, ButtonState, Hint, Segment};
use crate::code_text;
use crate::crash_log;
use crate::document_syntax::DocumentSyntax;
use crate::file_header::{self, Counterpart};
use crate::format;
use crate::horizontal_scroll::HorizontalScroll;
use crate::path_actions;
use crate::theme;

/// Where a merge of a git conflict's stages came from.
pub struct GitConflict {
    /// The working file, which the merged result replaces.
    pub path: PathBuf,
    /// Header titles for the left, base, and right panes.
    pub titles: [String; 3],
}

/// Lets the user resolve conflicts between three files and save the result.
pub struct MergeView {
    focus: FocusHandle,
    left: FileInfo,
    base: FileInfo,
    right: FileInfo,
    document: MergeDocument,
    /// Syntax state for the left, base, and right documents.
    syntax: [DocumentSyntax; 3],
    output: Vec<OutputLine>,
    /// Editor holding a manually edited copy of the output, while editing.
    editor: Option<Entity<InputState>>,
    horizontal: HorizontalScroll,
    /// Index into `document.conflicts` of the selected conflict.
    selected_conflict: Option<usize>,
    input_list: ListState,
    output_list: ListState,
    message: Option<(SharedString, Hsla)>,
    /// Whether long lines wrap.
    wrap: bool,
    /// Set when the inputs are a git conflict's stages rather than files.
    conflict: Option<GitConflict>,
}

impl MergeView {
    /// Creates the view; `infos` and `syntax` are ordered left, base, right.
    pub fn new(
        infos: [FileInfo; 3],
        syntax: [DocumentSyntax; 3],
        document: MergeDocument,
        cx: &mut Context<Self>,
    ) -> Self {
        let [left, base, right] = infos;
        let output = document.output_lines();
        let (document_rows, output_rows) = (document.rows.len(), output.len());
        let selected_conflict = (!document.conflicts.is_empty()).then_some(0);
        let max_columns = [&document.left, &document.base, &document.right]
            .into_iter()
            .map(format::max_display_columns)
            .max()
            .unwrap_or(0);
        Self {
            focus: cx.focus_handle(),
            left,
            base,
            right,
            document,
            syntax,
            output,
            editor: None,
            horizontal: HorizontalScroll::new(max_columns),
            selected_conflict,
            input_list: ListState::new(document_rows, ListAlignment::Top, px(300.0)),
            output_list: ListState::new(output_rows, ListAlignment::Top, px(300.0)),
            message: None,
            wrap: false,
            conflict: None,
        }
    }

    /// Marks the inputs as the stages of `conflict`: the panes are titled
    /// for the stages, and saving suggests the working file.
    pub fn with_git_conflict(mut self, conflict: Option<GitConflict>) -> Self {
        self.conflict = conflict;
        self
    }

    /// Sets whether long lines wrap.
    pub fn with_wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Turns line wrapping on or off in all panes, including the editor.
    pub fn set_wrap(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.wrap == wrap {
            return;
        }
        self.wrap = wrap;
        self.horizontal.reset();
        for list in [&self.input_list, &self.output_list] {
            let top = list.logical_scroll_top();
            list.reset(list.item_count());
            list.scroll_to(top);
        }
        if let Some(editor) = &self.editor {
            editor.update(cx, |state, cx| state.set_soft_wrap(wrap, window, cx));
        }
        cx.notify();
    }

    /// One-line description for the status bar.
    pub fn summary(&self) -> String {
        format!(
            "{} conflicts · {} unresolved · {} regions merged automatically",
            self.document.conflicts.len(),
            self.document.unresolved_count(),
            self.document
                .regions
                .iter()
                .filter(
                    |r| r.kind != MergeRegionKind::Unchanged && r.kind != MergeRegionKind::Conflict
                )
                .count(),
        )
    }

    pub fn next_difference(&mut self, cx: &mut Context<Self>) {
        let count = self.document.conflicts.len();
        if count > 0 {
            let next = self.selected_conflict.map_or(0, |c| (c + 1).min(count - 1));
            self.select_conflict(next, cx);
        }
    }

    pub fn prev_difference(&mut self, cx: &mut Context<Self>) {
        if !self.document.conflicts.is_empty() {
            let prev = self.selected_conflict.map_or(0, |c| c.saturating_sub(1));
            self.select_conflict(prev, cx);
        }
    }

    fn select_conflict(&mut self, conflict: usize, cx: &mut Context<Self>) {
        self.selected_conflict = Some(conflict);
        let region = self.document.conflicts[conflict];
        let near =
            |row: usize| ListOffset { item_ix: row.saturating_sub(5), offset_in_item: px(0.0) };
        self.input_list.scroll_to(near(self.document.region_first_row[region]));
        if let Some(line) = self.output_row_of_region(region) {
            self.output_list.scroll_to(near(line));
        }
        cx.notify();
    }

    /// First output line produced by `region`, if any.
    fn output_row_of_region(&self, region: usize) -> Option<usize> {
        let mut row = 0;
        for (index, (merge_region, resolution)) in
            self.document.regions.iter().zip(&self.document.resolutions).enumerate()
        {
            if index == region {
                return Some(row);
            }
            row += match resolution {
                Resolution::Base => merge_region.base.len(),
                Resolution::Left => merge_region.left.len(),
                Resolution::Right => merge_region.right.len(),
                Resolution::LeftThenRight | Resolution::RightThenLeft => {
                    merge_region.left.len() + merge_region.right.len()
                }
                Resolution::Unresolved => merge_region.left.len() + merge_region.right.len() + 3,
            };
        }
        None
    }

    fn resolve(&mut self, resolution: Resolution, cx: &mut Context<Self>) {
        let Some(conflict) = self.selected_conflict else {
            return;
        };
        let region = self.document.conflicts[conflict];
        self.document.resolutions[region] = resolution;
        self.output = self.document.output_lines();
        let top = self.output_list.logical_scroll_top();
        self.output_list.reset(self.output.len());
        self.output_list.scroll_to(top);
        self.message = None;
        // Move on to the next unresolved conflict, if there is one.
        let next_unresolved = (0..self.document.conflicts.len())
            .map(|offset| (conflict + 1 + offset) % self.document.conflicts.len())
            .find(|&c| {
                self.document.resolutions[self.document.conflicts[c]] == Resolution::Unresolved
            });
        match next_unresolved {
            Some(next) => self.select_conflict(next, cx),
            None => cx.notify(),
        }
    }

    fn on_row_click(&mut self, row: usize, cx: &mut Context<Self>) {
        let region = self.document.rows[row].region as usize;
        if let Some(conflict) = self.document.conflicts.iter().position(|&c| c == region) {
            self.selected_conflict = Some(conflict);
            cx.notify();
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let directory = self.base.path.parent().map(PathBuf::from).unwrap_or_default();
        let file_name = self
            .base
            .path
            .file_name()
            .map_or_else(|| "merged".to_owned(), |name| name.to_string_lossy().into_owned());
        // A git conflict is resolved in place; other merges get a new file.
        let (suggested, resolving_conflict) = match &self.conflict {
            Some(_) => (file_name, true),
            None => (format!("{file_name}.merged"), false),
        };
        let receiver = cx.prompt_for_new_path(&directory, Some(&suggested));
        let (text, unresolved) = match &self.editor {
            Some(editor) => {
                let mut edited = editor.read(cx).value().to_string();
                // Keep the base file's trailing newline convention.
                if self.document.base.text().ends_with('\n') && !edited.ends_with('\n') {
                    edited.push('\n');
                }
                let text = if self.document.base.text().contains("\r\n") {
                    edited.replace("\r\n", "\n").replace('\n', "\r\n")
                } else {
                    edited
                };
                (text, 0)
            }
            None => (self.document.merged_text(), self.document.unresolved_count()),
        };
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = receiver.await else {
                return;
            };
            let write_result = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move { std::fs::write(&path, text) }
                })
                .await;
            let message = match write_result {
                Ok(()) if unresolved > 0 => (
                    SharedString::from(format!(
                        "Saved {} with {unresolved} unresolved conflict(s) marked",
                        path.display()
                    )),
                    theme::yellow(),
                ),
                Ok(()) if resolving_conflict => (
                    SharedString::from(format!(
                        "Saved {}. Run git add to mark the conflict resolved.",
                        path.display()
                    )),
                    theme::green(),
                ),
                Ok(()) => (SharedString::from(format!("Saved {}", path.display())), theme::green()),
                Err(e) => {
                    crash_log::record_error("Saving merged output", &e.to_string());
                    (
                        SharedString::from(format!("Could not save {}: {e}", path.display())),
                        theme::red(),
                    )
                }
            };
            this.update(cx, |this, cx| {
                this.message = Some(message);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Switches the output pane to an editor seeded with the current merge.
    fn start_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.document.merged_text().replace("\r\n", "\n");
        let editor = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .code_editor("text")
                .line_number(true)
                .soft_wrap(self.wrap);
            state.set_value(text, window, cx);
            state
        });
        editor.update(cx, |state, cx| state.focus(window, cx));
        self.editor = Some(editor);
        self.message = Some((
            SharedString::from("Editing the merged output; resolution buttons are paused"),
            theme::accent(),
        ));
        cx.notify();
    }

    /// Drops manual edits and returns to the generated output.
    fn discard_edits(&mut self, cx: &mut Context<Self>) {
        self.editor = None;
        self.message = None;
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        if self.horizontal.handle(event) {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn selected_region(&self) -> Option<usize> {
        self.selected_conflict.map(|c| self.document.conflicts[c])
    }

    fn render_input_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let selected_region = self.selected_region();
        {
            let row = self.document.rows[index];
            let region = &self.document.regions[row.region as usize];
            let selected = selected_region == Some(row.region as usize);
            let resolution = self.document.resolutions[row.region as usize];
            let (left_bg, base_bg, right_bg) = region_backgrounds(region.kind, selected);
            let marker = match (region.kind, resolution) {
                (MergeRegionKind::Conflict, Resolution::Unresolved) => theme::yellow(),
                (MergeRegionKind::Conflict, _) => theme::green(),
                _ => theme::base(),
            };
            self.row_height(div().id(("merge-row", index)).flex().flex_row().w_full())
                .font_family(theme::MONO_FONT)
                .line_height(theme::row_height())
                .text_size(theme::code_text_size())
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.on_row_click(index, cx)),
                )
                .on_scroll_wheel(
                    cx.listener(|this, event: &ScrollWheelEvent, _, cx| this.on_scroll(event, cx)),
                )
                .child(div().flex_none().w(px(3.0)).bg(marker))
                .child(self.merge_cell(row.left, 0, left_bg))
                .child(div().flex_none().w(px(1.0)).bg(theme::surface()))
                .child(self.merge_cell(row.base, 1, base_bg))
                .child(div().flex_none().w(px(1.0)).bg(theme::surface()))
                .child(self.merge_cell(row.right, 2, right_bg))
                .into_any_element()
        }
    }

    /// Fixes a row to one line, or lets it grow when lines wrap.
    fn row_height<E: Styled>(&self, row: E) -> E {
        if self.wrap { row.min_h(theme::row_height()) } else { row.h(theme::row_height()) }
    }

    fn render_output_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        {
            let line = self.output[index];
            let text = format::display_line(self.document.output_text(line));
            let (background, color, tokens) = match line {
                OutputLine::Base(i) => {
                    (theme::base(), theme::text(), self.syntax[1].tokens(i as usize, &text))
                }
                OutputLine::Left(i) => {
                    (theme::deleted_bg(), theme::text(), self.syntax[0].tokens(i as usize, &text))
                }
                OutputLine::Right(i) => {
                    (theme::inserted_bg(), theme::text(), self.syntax[2].tokens(i as usize, &text))
                }
                OutputLine::ConflictStart
                | OutputLine::ConflictSeparator
                | OutputLine::ConflictEnd => (theme::conflict_bg(), theme::yellow(), Vec::new()),
            };
            self.row_height(div().flex().flex_row().w_full())
                .font_family(theme::MONO_FONT)
                .text_size(theme::code_text_size())
                .line_height(theme::row_height())
                .bg(background)
                .on_scroll_wheel(
                    cx.listener(|this, event: &ScrollWheelEvent, _, cx| this.on_scroll(event, cx)),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(52.0))
                        .pr_2()
                        .flex()
                        .justify_end()
                        .bg(theme::mantle())
                        .text_color(theme::gutter())
                        .child((index + 1).to_string()),
                )
                .child(text_column(
                    code_text::code_text(text, &tokens, &[]),
                    color,
                    self.horizontal.offset(),
                    self.wrap,
                ))
                .into_any_element()
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let editing = self.editor.is_some();
        let has_selection = if self.selected_conflict.is_some() && !editing {
            ButtonState::Enabled
        } else {
            ButtonState::Disabled
        };
        let has_conflicts = if self.document.conflicts.is_empty() {
            ButtonState::Disabled
        } else {
            ButtonState::Enabled
        };
        let resolve = |id: &'static str,
                       label: &'static str,
                       hint: &'static str,
                       resolution: Resolution,
                       cx: &mut Context<Self>| {
            Segment::new(
                id,
                label,
                has_selection,
                Hint::text(hint),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.resolve(resolution, cx);
                }),
            )
        };
        let status = match &self.message {
            Some((message, color)) => (message.clone(), *color),
            None => {
                let unresolved = self.document.unresolved_count();
                if unresolved == 0 {
                    (SharedString::from("All conflicts resolved"), theme::green())
                } else {
                    (
                        SharedString::from(format!("{unresolved} unresolved conflict(s)")),
                        theme::yellow(),
                    )
                }
            }
        };
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
            .child(button::segmented(vec![
                Segment::new(
                    "prev-conflict",
                    IconName::ArrowUp,
                    has_conflicts,
                    Hint::action("Previous conflict", &PrevDifference),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.prev_difference(cx)),
                ),
                Segment::new(
                    "next-conflict",
                    IconName::ArrowDown,
                    has_conflicts,
                    Hint::action("Next conflict", &NextDifference),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.next_difference(cx)),
                ),
            ]))
            .child(button::segmented(vec![
                resolve("take-left", "Left", "Use the left version", Resolution::Left, cx),
                resolve("take-right", "Right", "Use the right version", Resolution::Right, cx),
                resolve(
                    "take-left-right",
                    "Left + Right",
                    "Use the left version, then the right",
                    Resolution::LeftThenRight,
                    cx,
                ),
                resolve(
                    "take-right-left",
                    "Right + Left",
                    "Use the right version, then the left",
                    Resolution::RightThenLeft,
                    cx,
                ),
                resolve("take-base", "Base", "Keep the base version", Resolution::Base, cx),
                resolve(
                    "unresolve",
                    "Unresolve",
                    "Mark the conflict as unresolved",
                    Resolution::Unresolved,
                    cx,
                ),
            ]))
            .child(if editing {
                button::button(
                    "discard-edits",
                    "Discard Edits",
                    ButtonState::Enabled,
                    cx.listener(|this, _: &ClickEvent, _, cx| this.discard_edits(cx)),
                )
            } else {
                button::button(
                    "edit-output",
                    "Edit Output",
                    ButtonState::Enabled,
                    cx.listener(|this, _: &ClickEvent, window, cx| this.start_editing(window, cx)),
                )
            })
            .child(
                button::button(
                    "save-merged",
                    "Save Merged…",
                    ButtonState::Enabled,
                    cx.listener(|this, _: &ClickEvent, _, cx| this.save(cx)),
                )
                .px_3()
                .font_weight(FontWeight::SEMIBOLD)
                .bg(theme::accent())
                .text_color(theme::crust())
                .hover(|style| style.bg(theme::text()).text_color(theme::crust())),
            )
            .child(div().flex_1())
            .child(div().text_size(theme::ui_text_size()).text_color(status.1).child(status.0))
    }
}

/// Backgrounds for the left, base, and right cells of a region.
fn region_backgrounds(kind: MergeRegionKind, selected: bool) -> (Hsla, Hsla, Hsla) {
    let plain = theme::base();
    match kind {
        MergeRegionKind::Unchanged => (plain, plain, plain),
        MergeRegionKind::LeftChanged => (theme::deleted_bg(), plain, plain),
        MergeRegionKind::RightChanged => (plain, plain, theme::inserted_bg()),
        MergeRegionKind::BothChanged => (theme::both_changed_bg(), plain, theme::both_changed_bg()),
        MergeRegionKind::Conflict => {
            let bg = if selected { theme::conflict_selected_bg() } else { theme::conflict_bg() };
            (bg, bg, bg)
        }
    }
}

/// The text column of a row: wrapped, or shifted by the horizontal scroll
/// offset.
fn text_column(text: gpui::StyledText, color: Hsla, offset: Pixels, wrap: bool) -> Div {
    let inner = div().text_color(color).child(text);
    let inner =
        if wrap { inner.whitespace_normal() } else { inner.ml(-offset).whitespace_nowrap() };
    div().flex_1().min_w_0().pl_2().overflow_hidden().child(inner)
}

impl MergeView {
    /// One input column of a row; `side` is 0 for left, 1 for base, 2 for right.
    fn merge_cell(&self, line: Option<u32>, side: usize, background: Hsla) -> Div {
        let document: &TextDocument = match side {
            0 => &self.document.left,
            1 => &self.document.base,
            _ => &self.document.right,
        };
        let (number, text) = match line {
            Some(index) => {
                ((index + 1).to_string(), format::display_line(document.line(index as usize)))
            }
            None => (String::new(), String::new()),
        };
        let tokens =
            line.map_or_else(Vec::new, |index| self.syntax[side].tokens(index as usize, &text));
        let background = if line.is_none() { theme::filler_bg() } else { background };
        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .bg(background)
            .child(
                div()
                    .flex_none()
                    .w(px(44.0))
                    .pr_2()
                    .flex()
                    .justify_end()
                    .bg(theme::mantle())
                    .text_color(theme::gutter())
                    .child(number),
            )
            .child(text_column(
                code_text::code_text(text, &tokens, &[]),
                theme::text(),
                self.horizontal.offset(),
                self.wrap,
            ))
    }
}

impl MergeView {
    /// Headers of the left, base, and right panes, with dividers between.
    fn input_headers(&self) -> Vec<Div> {
        let divider = || div().w(px(1.0)).bg(theme::surface());
        let headers = match &self.conflict {
            Some(conflict) => {
                let infos = [&self.left, &self.base, &self.right];
                let titled = |index: usize| {
                    file_header::titled_header(
                        conflict.titles[index].clone(),
                        file_header::folder_label(&conflict.path),
                        format::format_size(infos[index].size),
                    )
                };
                [titled(0), titled(1), titled(2)]
            }
            None => [
                file_header::file_header(
                    "merge-left",
                    &self.left,
                    Some(Counterpart { info: &self.base, name: "base" }),
                ),
                file_header::titled_header(
                    format!("Base: {}", format::file_name(&self.base.path)),
                    file_header::folder_label(&self.base.path),
                    format::format_size(self.base.size),
                ),
                file_header::file_header(
                    "merge-right",
                    &self.right,
                    Some(Counterpart { info: &self.base, name: "base" }),
                ),
            ],
        };
        let [left, base, right] = headers;
        vec![left, divider(), base, divider(), right]
    }
}

impl Render for MergeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_toolbar(cx))
            .child(div().flex().flex_row().children(self.input_headers()))
            .child(
                div().flex_grow().flex_basis(px(0.0)).min_h_0().child(
                    path_actions::with_file_pair_menu(
                        div().flex().flex_col().size_full().child(
                            list(
                                self.input_list.clone(),
                                cx.processor(|this, index: usize, _window, cx| {
                                    this.render_input_row(index, cx)
                                }),
                            )
                            .flex_1()
                            .bg(theme::base()),
                        ),
                        &self.focus,
                        Some(self.left.path.clone()),
                        Some(self.right.path.clone()),
                    ),
                ),
            )
            .child(div().flex().flex_row().child(file_header::header_bar(
                if self.editor.is_some() { "Merged output (editing)" } else { "Merged output" },
                format!("{} lines", self.output.len()),
            )))
            .child(match &self.editor {
                Some(editor) => div()
                    .flex_grow()
                    .flex_basis(px(0.0))
                    .min_h_0()
                    .font_family(theme::MONO_FONT)
                    .text_size(theme::code_text_size())
                    .child(Input::new(editor).h_full().appearance(false))
                    .into_any_element(),
                None => list(
                    self.output_list.clone(),
                    cx.processor(|this, index: usize, _window, cx| {
                        this.render_output_row(index, cx)
                    }),
                )
                .flex_grow()
                .flex_basis(px(0.0))
                .bg(theme::base())
                .into_any_element(),
            })
    }
}
