//! Three-way merge view: left / base / right panes plus a merged output pane.

use std::ops::Range;
use std::path::PathBuf;

use gpui::{
    AppContext, ClickEvent, Context, Div, Entity, FocusHandle, Hsla, InteractiveElement,
    IntoElement, ParentElement, Pixels, Render, ScrollStrategy, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, div, px, uniform_list,
};
use gpui_component::input::{Input, InputState};

use compare_core::{FileInfo, MergeDocument, OutputLine, Resolution, TextDocument};
use compare_diff::MergeRegionKind;

use crate::button::{self, ButtonState};
use crate::code_text;
use crate::crash_log;
use crate::document_syntax::DocumentSyntax;
use crate::file_header;
use crate::format;
use crate::horizontal_scroll::HorizontalScroll;
use crate::path_actions;
use crate::theme;

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
    input_scroll: UniformListScrollHandle,
    output_scroll: UniformListScrollHandle,
    message: Option<(SharedString, Hsla)>,
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
            input_scroll: UniformListScrollHandle::new(),
            output_scroll: UniformListScrollHandle::new(),
            message: None,
        }
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
        self.input_scroll
            .scroll_to_item(self.document.region_first_row[region], ScrollStrategy::Center);
        if let Some(line) = self.output_row_of_region(region) {
            self.output_scroll.scroll_to_item(line, ScrollStrategy::Center);
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
        let suggested = format!(
            "{}.merged",
            self.base
                .path
                .file_name()
                .map_or_else(|| "merged".to_owned(), |name| name.to_string_lossy().into_owned(),)
        );
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
            let mut state = InputState::new(window, cx).code_editor("text").line_number(true);
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

    fn render_input_rows(
        &self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let selected_region = self.selected_region();
        range
            .map(|index| {
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
                div()
                    .id(("merge-row", index))
                    .flex()
                    .flex_row()
                    .w_full()
                    .h(theme::row_height())
                    .font_family(theme::MONO_FONT)
                    .text_size(theme::code_text_size())
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.on_row_click(index, cx)
                        }),
                    )
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        this.on_scroll(event, cx)
                    }))
                    .child(div().flex_none().w(px(3.0)).h_full().bg(marker))
                    .child(self.merge_cell(row.left, 0, left_bg))
                    .child(div().flex_none().w(px(1.0)).h_full().bg(theme::surface()))
                    .child(self.merge_cell(row.base, 1, base_bg))
                    .child(div().flex_none().w(px(1.0)).h_full().bg(theme::surface()))
                    .child(self.merge_cell(row.right, 2, right_bg))
            })
            .collect()
    }

    fn render_output_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Div> {
        range
            .map(|index| {
                let line = self.output[index];
                let text = format::display_line(self.document.output_text(line));
                let (background, color, tokens) = match line {
                    OutputLine::Base(i) => {
                        (theme::base(), theme::text(), self.syntax[1].tokens(i as usize, &text))
                    }
                    OutputLine::Left(i) => (
                        theme::deleted_bg(),
                        theme::text(),
                        self.syntax[0].tokens(i as usize, &text),
                    ),
                    OutputLine::Right(i) => (
                        theme::inserted_bg(),
                        theme::text(),
                        self.syntax[2].tokens(i as usize, &text),
                    ),
                    OutputLine::ConflictStart
                    | OutputLine::ConflictSeparator
                    | OutputLine::ConflictEnd => {
                        (theme::conflict_bg(), theme::yellow(), Vec::new())
                    }
                };
                div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .h(theme::row_height())
                    .font_family(theme::MONO_FONT)
                    .text_size(theme::code_text_size())
                    .bg(background)
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        this.on_scroll(event, cx)
                    }))
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
                    ))
            })
            .collect()
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
        let resolve_button = |id: &'static str,
                              label: &'static str,
                              resolution: Resolution,
                              cx: &mut Context<Self>| {
            button::button(
                id,
                label,
                has_selection,
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
            .items_center()
            .gap_1()
            .px_2()
            .py(px(3.0))
            .bg(theme::mantle())
            .border_b_1()
            .border_color(theme::surface())
            .child(button::button(
                "prev-conflict",
                "◀ Prev Conflict",
                has_conflicts,
                cx.listener(|this, _: &ClickEvent, _, cx| this.prev_difference(cx)),
            ))
            .child(button::button(
                "next-conflict",
                "Next Conflict ▶",
                has_conflicts,
                cx.listener(|this, _: &ClickEvent, _, cx| this.next_difference(cx)),
            ))
            .child(button::separator())
            .child(resolve_button("take-left", "Take Left", Resolution::Left, cx))
            .child(resolve_button("take-right", "Take Right", Resolution::Right, cx))
            .child(resolve_button(
                "take-left-right",
                "Left then Right",
                Resolution::LeftThenRight,
                cx,
            ))
            .child(resolve_button(
                "take-right-left",
                "Right then Left",
                Resolution::RightThenLeft,
                cx,
            ))
            .child(resolve_button("take-base", "Take Base", Resolution::Base, cx))
            .child(resolve_button("unresolve", "Unresolve", Resolution::Unresolved, cx))
            .child(button::separator())
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
                    "✎ Edit Output",
                    ButtonState::Enabled,
                    cx.listener(|this, _: &ClickEvent, window, cx| this.start_editing(window, cx)),
                )
            })
            .child(button::button(
                "save-merged",
                "💾 Save Merged…",
                ButtonState::Enabled,
                cx.listener(|this, _: &ClickEvent, _, cx| this.save(cx)),
            ))
            .child(div().flex_1())
            .child(div().text_size(px(11.0)).text_color(status.1).child(status.0))
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

/// The text column of a row, shifted by the horizontal scroll offset.
fn text_column(text: gpui::StyledText, color: Hsla, offset: Pixels) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .pl_2()
        .overflow_hidden()
        .child(div().ml(-offset).whitespace_nowrap().text_color(color).child(text))
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
            .h_full()
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
            ))
    }
}

impl Render for MergeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(file_header::file_header(&self.left))
                    .child(div().w(px(1.0)).bg(theme::surface()))
                    .child(file_header::header_bar(
                        format!("Base: {}", format::path_label(&self.base.path)),
                        format::format_size(self.base.size),
                    ))
                    .child(div().w(px(1.0)).bg(theme::surface()))
                    .child(file_header::file_header(&self.right)),
            )
            .child(
                div().flex_grow().flex_basis(px(0.0)).min_h_0().child(
                    path_actions::with_file_pair_menu(
                        div().flex().flex_col().size_full().child(
                            uniform_list(
                                "merge-input-rows",
                                self.document.rows.len(),
                                cx.processor(|this, range: Range<usize>, _window, cx| {
                                    this.render_input_rows(range, cx)
                                }),
                            )
                            .track_scroll(self.input_scroll.clone())
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
                None => uniform_list(
                    "merge-output-rows",
                    self.output.len(),
                    cx.processor(|this, range: Range<usize>, _window, cx| {
                        this.render_output_rows(range, cx)
                    }),
                )
                .track_scroll(self.output_scroll.clone())
                .flex_grow()
                .flex_basis(px(0.0))
                .bg(theme::base())
                .into_any_element(),
            })
    }
}
