//! Drawing the comparison: aligned rows of editable lines.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, Div, ElementInputHandler, Hsla,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Render, ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled,
    WeakEntity, Window, canvas, div, list, px,
};

use compare_core::{DiffSide, RowKind};
use compare_diff::InlineDiff;

use super::TextDiffView;
use super::editing::Motion;
use super::git_versions::Step;
use gpui_component::IconName;
use gpui_component::tooltip::Tooltip;

use crate::actions::{
    Backspace, CloseFind, CollapseSelection, Copy, CopyBlockToLeft, CopyBlockToRight,
    CopyLineToLeft, CopyLineToRight, Cut, DIFF_EDITOR_CONTEXT, DeleteForward, DeleteToLineStart,
    DeleteWordLeft, DeleteWordRight, FindNext, FindPrev, Indent, InsertLine, MoveDocEnd,
    MoveDocStart, MoveDown, MoveLeft, MoveLineEnd, MoveLineStart, MovePageDown, MovePageUp,
    MoveRight, MoveUp, MoveWordLeft, MoveWordRight, Newline, OpenFind, OpenFolderLeft,
    OpenFolderRight, OpenLeft, OpenReplace, OpenRight, Paste, Redo, ReplaceAll, ReplaceNext,
    SelectAll, SelectDocEnd, SelectDocStart, SelectDown, SelectLeft, SelectLineEnd,
    SelectLineStart, SelectPageDown, SelectPageUp, SelectRight, SelectUp, SelectWordLeft,
    SelectWordRight, ShowCharacterPalette, Undo,
};
use crate::button::{self, ButtonState, Segment};
use crate::code_text::{self, Mark};
use crate::editor_line::EditorLine;
use crate::file_header::{self, Counterpart};
use crate::format;
use crate::path_actions::{self, MenuItems};
use crate::theme;

use super::motion;
use compare_core::EditKind;

/// Width of the line-number gutter.
pub(super) const GUTTER_WIDTH: f32 = 48.0;

/// Width of the change marker on each outer edge of a row.
pub(super) const MARKER_WIDTH: f32 = 3.0;

/// Width of the dividers on either side of the center column.
pub(super) const DIVIDER_WIDTH: f32 = 1.0;

/// Width of the column between the sides that holds the copy arrows.
pub(super) const CENTER_WIDTH: f32 = 48.0;

/// Padding between the line numbers and the text.
pub(super) const TEXT_PADDING: f32 = 8.0;

/// Width of a block-copy arrow (« or ») in the center column.
const BLOCK_ARROW_WIDTH: f32 = 13.0;

/// Width of a line-copy arrow (‹ or ›) in the center column.
const LINE_ARROW_WIDTH: f32 = 11.0;

/// Group name for showing a row's line-copy arrows on hover.
const ROW_GROUP: &str = "diff-row";

/// A copy arrow in the center column.
struct CopyArrow {
    id: &'static str,
    glyph: &'static str,
    /// True for a single line, false for a whole block.
    line: bool,
    /// The side copied from.
    from: DiffSide,
    hint: &'static str,
    action: Box<dyn gpui::Action>,
}

impl CopyArrow {
    fn block(from: DiffSide) -> Self {
        match from {
            DiffSide::Left => Self {
                id: "copy-block-right",
                glyph: "»",
                line: false,
                from,
                hint: "Copy section to right",
                action: Box::new(CopyBlockToRight),
            },
            DiffSide::Right => Self {
                id: "copy-block-left",
                glyph: "«",
                line: false,
                from,
                hint: "Copy section to left",
                action: Box::new(CopyBlockToLeft),
            },
        }
    }

    fn line(from: DiffSide) -> Self {
        match from {
            DiffSide::Left => Self {
                id: "copy-line-right",
                glyph: "›",
                line: true,
                from,
                hint: "Copy line to right",
                action: Box::new(CopyLineToRight),
            },
            DiffSide::Right => Self {
                id: "copy-line-left",
                glyph: "‹",
                line: true,
                from,
                hint: "Copy line to left",
                action: Box::new(CopyLineToLeft),
            },
        }
    }
}

/// A 1px divider between a side and the center column.
fn divider() -> Div {
    div().flex_none().w(px(DIVIDER_WIDTH)).bg(theme::surface())
}

/// How one side of a row is drawn.
struct CellStyle<'a> {
    background: Hsla,
    /// Changed words, as byte ranges of the display text.
    emphasis: &'a [Range<usize>],
    emphasis_color: Hsla,
}

impl TextDiffView {
    fn render_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = self.diff.rows()[index];
        let display = |side: DiffSide| {
            side.line_of(row)
                .map(|line| format::display_line(self.side(side).buffer.line(line as usize)))
        };
        let (left_text, right_text) = (display(DiffSide::Left), display(DiffSide::Right));
        let inline = match (&left_text, &right_text) {
            (Some(l), Some(r)) if row.kind == RowKind::Changed => compare_diff::diff_inline(l, r),
            _ => InlineDiff::default(),
        };
        let (left_bg, right_bg) = match row.kind {
            RowKind::Equal => (theme::base(), theme::base()),
            RowKind::Changed => (theme::deleted_bg(), theme::inserted_bg()),
            RowKind::Deleted => (theme::deleted_bg(), theme::filler_bg()),
            RowKind::Inserted => (theme::filler_bg(), theme::inserted_bg()),
        };
        let is_current = self.current_change_rows().is_some_and(|rows| rows.contains(&index));
        let marker = if is_current { theme::accent() } else { theme::base() };
        let focused = self.focus.is_focused(window);

        let left = self.render_cell(
            DiffSide::Left,
            index,
            left_text,
            CellStyle {
                background: left_bg,
                emphasis: &inline.left,
                emphasis_color: theme::deleted_emphasis(),
            },
            focused,
            cx,
        );
        let right = self.render_cell(
            DiffSide::Right,
            index,
            right_text,
            CellStyle {
                background: right_bg,
                emphasis: &inline.right,
                emphasis_color: theme::inserted_emphasis(),
            },
            focused,
            cx,
        );
        let row_div = div().flex().flex_row().w_full();
        let row_div = if self.wrap {
            row_div.min_h(theme::row_height())
        } else {
            row_div.h(theme::row_height()).on_scroll_wheel(cx.listener(
                |this, event: &ScrollWheelEvent, _, cx| {
                    if this.horizontal.handle(event) {
                        cx.stop_propagation();
                        cx.notify();
                    }
                },
            ))
        };
        row_div
            .group(ROW_GROUP)
            .child(div().flex_none().w(px(MARKER_WIDTH)).bg(marker))
            .child(left)
            .child(divider())
            .child(self.render_center(index, row.kind, cx))
            .child(divider())
            .child(right)
            .child(div().flex_none().w(px(MARKER_WIDTH)).bg(marker))
            .into_any_element()
    }

    /// The column between the sides: « and » copy a whole changed block (on
    /// its first row); ‹ and › copy one line (faint until the row is
    /// hovered).
    fn render_center(&self, index: usize, kind: RowKind, cx: &mut Context<Self>) -> Div {
        let column = div()
            .flex_none()
            .w(px(CENTER_WIDTH))
            .flex()
            .flex_row()
            .bg(theme::mantle())
            // Clicks here are for the arrows, not for placing the cursor.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        if kind == RowKind::Equal {
            return column;
        }
        let starts_change = self.starts_change(index);
        let writable = |side: DiffSide| self.side(side).read_only.is_none();
        let arrow = |copy: CopyArrow, cx: &mut Context<Self>| {
            let width = if copy.line { LINE_ARROW_WIDTH } else { BLOCK_ARROW_WIDTH };
            let slot = div().flex_none().w(px(width)).flex().items_center().justify_center();
            let shown = copy.line || starts_change;
            if !shown || !writable(copy.from.other()) {
                return slot;
            }
            let (from, line) = (copy.from, copy.line);
            let button = div()
                .id((copy.id, index))
                .h_full()
                .min_h(px(16.0))
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(theme::ui_text_size())
                .cursor_pointer()
                .child(copy.glyph)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.focus);
                    if line {
                        this.copy_line(index, from, cx);
                    } else {
                        this.copy_block(index, from, cx);
                    }
                }));
            let button = if line {
                button
                    .text_color(theme::subtext())
                    .opacity(0.35)
                    .group_hover(ROW_GROUP, |style| style.opacity(1.0))
                    .hover(|style| style.text_color(theme::text()))
            } else {
                button
                    .h(px(16.0))
                    .rounded(px(3.0))
                    .bg(theme::surface())
                    .text_color(theme::text())
                    .hover(|style| style.bg(theme::surface_bright()))
            };
            let hint = button::Hint::action(copy.hint, copy.action.as_ref());
            slot.child(button::with_hint(button, hint))
        };
        column
            .child(arrow(CopyArrow::block(DiffSide::Right), cx))
            .child(arrow(CopyArrow::line(DiffSide::Right), cx))
            .child(arrow(CopyArrow::line(DiffSide::Left), cx))
            .child(arrow(CopyArrow::block(DiffSide::Left), cx))
    }

    /// One side of a row: line-number gutter plus the line's text, with the
    /// selection and cursor drawn over it.
    fn render_cell(
        &self,
        side: DiffSide,
        row: usize,
        display: Option<String>,
        style: CellStyle<'_>,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let line = side.line_of(self.diff.rows()[row]).map(|line| line as usize);
        let number = line.map_or_else(String::new, |line| (line + 1).to_string());
        let content = line
            .zip(display)
            .map(|(line, display)| self.render_line(side, line, display, &style, focused));

        let text = div().text_color(theme::text()).children(content);
        let text = if self.wrap {
            text.whitespace_normal()
        } else {
            text.ml(-self.horizontal.offset()).whitespace_nowrap()
        };
        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .bg(style.background)
            .font_family(theme::MONO_FONT)
            .text_size(theme::code_text_size())
            .line_height(theme::row_height())
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.mouse_down(side, row, event, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.mouse_down_right(side, row, event, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                this.mouse_drag(side, row, event, cx);
            }))
            .child(self.render_gutter(side, line, number))
            .child(div().flex_1().min_w_0().pl(px(TEXT_PADDING)).overflow_hidden().child(text))
    }

    /// The line-number gutter. In a git history view, hovering the number
    /// shows the line's blame.
    fn render_gutter(&self, side: DiffSide, line: Option<usize>, number: String) -> AnyElement {
        let gutter = div()
            .flex_none()
            .w(px(GUTTER_WIDTH))
            .pr(px(TEXT_PADDING))
            .flex()
            .justify_end()
            .text_color(theme::gutter())
            .bg(theme::mantle())
            .child(number);
        let Some((line, hint)) = line.and_then(|line| Some((line, self.blame_hint(side, line)?)))
        else {
            return gutter.into_any_element();
        };
        let id = match side {
            DiffSide::Left => "blame-left",
            DiffSide::Right => "blame-right",
        };
        let hint = SharedString::from(hint);
        gutter
            .id((id, line))
            .hover(|style| style.text_color(theme::text()))
            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
            .into_any_element()
    }

    /// The text of `line` on `side` with syntax colors, changed words,
    /// selection, input-method underline, and cursor.
    fn render_line(
        &self,
        side: DiffSide,
        line: usize,
        mut display: String,
        style: &CellStyle<'_>,
        focused: bool,
    ) -> EditorLine {
        let state = self.side(side);
        let text = state.buffer.line(line);
        let tokens = state.syntax.tokens(line, &display);
        let mut marks: Vec<Mark> = style
            .emphasis
            .iter()
            .map(|range| Mark::background(range.clone(), style.emphasis_color))
            .collect();

        if let Some(find) = &self.find {
            let current = find.current_match();
            for found in find.matches_on_line(side, line) {
                let start = if found.start.line == line { found.start.column } else { 0 };
                let end = if found.end.line == line { found.end.column } else { text.len() };
                let range =
                    format::to_display_offset(text, start)..format::to_display_offset(text, end);
                let is_current = current.is_some_and(|(s, m)| s == side && m == found);
                marks.push(Mark {
                    range,
                    background: Some(if is_current {
                        theme::find_current()
                    } else {
                        theme::find_match()
                    }),
                    underline: Some(theme::yellow()),
                });
            }
        }

        let is_active = side == self.active;
        let selection = state.selection.range();
        if !state.selection.is_empty() && selection.start.line <= line && line <= selection.end.line
        {
            let start = if selection.start.line == line {
                format::to_display_offset(text, selection.start.column)
            } else {
                0
            };
            let end = if selection.end.line == line {
                format::to_display_offset(text, selection.end.column)
            } else {
                // The selection includes the line break: show it as a space.
                display.push(' ');
                display.len()
            };
            let color = if is_active { theme::selection() } else { theme::selection_inactive() };
            marks.push(Mark::background(start..end, color));
        }

        let head = state.selection.head;
        let has_cursor = is_active && head.line == line;
        if has_cursor && let Some(marked) = &state.marked {
            let range = format::to_display_offset(text, marked.start)
                ..format::to_display_offset(text, marked.end);
            marks.push(Mark::underline(range, theme::text()));
        }

        let mut element = EditorLine::new(code_text::code_text(display, &tokens, &marks));
        if has_cursor && focused {
            element = element.cursor(format::to_display_offset(text, head.column), theme::cursor());
        }
        let layouts = self.layouts.clone();
        element.on_layout(Box::new(move |layout, _| {
            layouts.borrow_mut().insert((side, line), layout);
        }))
    }

    /// The file headers, each with an unsaved-changes marker and Save and
    /// Revert buttons.
    /// The file headers, laid out on the same grid as the rows: name and
    /// folder, details, and buttons to choose another file, save, or revert.
    fn render_headers(&self, cx: &mut Context<Self>) -> Div {
        let header = |side: DiffSide, cx: &mut Context<Self>| {
            let state = self.side(side);
            let modified = state.is_modified();
            let git = self.is_git_history();
            let name = if git { format::file_name(&state.info.path) } else { state.display_name() };
            let name = if modified { format!("● {name}") } else { name };
            let folder = match (self.version_label(side), &state.path) {
                (Some(version), _) => version,
                (None, Some(path)) => file_header::folder_label(path),
                (None, None) => "Not saved yet".to_owned(),
            };
            // A side has details when it is a file or a committed version.
            let has_details = |state: &super::side::SideState| {
                state.path.is_some() || (git && state.info.modified.is_some())
            };
            // Size and date compare with the other side when both have them.
            let other = self.side(side.other());
            let counterpart = has_details(other).then_some(Counterpart {
                info: &other.info,
                name: match side {
                    DiffSide::Left => "right file",
                    DiffSide::Right => "left file",
                },
            });
            let id = match side {
                DiffSide::Left => "header-left",
                DiffSide::Right => "header-right",
            };
            let details = div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .children(state.read_only.clone())
                .children((state.read_only.is_some() && has_details(state)).then_some("·"))
                .children(
                    has_details(state)
                        .then(|| file_header::file_details(id, &state.info, counterpart)),
                );
            let navigation = git.then(|| self.render_version_steps(side, cx));
            let pickers = (self.file_backed && !git).then(|| {
                let (file, folder): (Box<dyn gpui::Action>, Box<dyn gpui::Action>) = match side {
                    DiffSide::Left => (Box::new(OpenLeft), Box::new(OpenFolderLeft)),
                    DiffSide::Right => (Box::new(OpenRight), Box::new(OpenFolderRight)),
                };
                button::segmented(vec![
                    Segment::action(
                        IconName::File,
                        file,
                        ButtonState::Enabled,
                        "Choose another file",
                    ),
                    Segment::action(
                        IconName::FolderOpen,
                        folder,
                        ButtonState::Enabled,
                        "Choose a folder",
                    ),
                ])
            });
            let save = button::with_hint(
                button::button(
                    ("save-side", side.index()),
                    "Save",
                    ButtonState::enabled(modified && state.read_only.is_none()),
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        window.focus(&this.focus);
                        this.save(side, window, cx).detach();
                    }),
                ),
                button::Hint::text("Save this side"),
            );
            let revert = button::with_hint(
                button::button(
                    ("revert-side", side.index()),
                    "Revert",
                    ButtonState::enabled(modified && state.path.is_some()),
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        window.focus(&this.focus);
                        this.revert(side, cx);
                    }),
                ),
                button::Hint::text("Discard unsaved changes to this side"),
            );
            file_header::titled_header(name, folder, details)
                .children(navigation)
                .children(pickers)
                .child(save)
                .child(revert)
        };
        let spacer = |width: f32| {
            div()
                .flex_none()
                .w(px(width))
                .bg(theme::mantle())
                .border_b_1()
                .border_color(theme::surface())
        };
        div()
            .flex()
            .flex_row()
            .child(spacer(MARKER_WIDTH))
            .child(header(DiffSide::Left, cx))
            .child(divider())
            .child(spacer(CENTER_WIDTH))
            .child(divider())
            .child(header(DiffSide::Right, cx))
            .child(spacer(MARKER_WIDTH))
    }

    /// Older and newer buttons that step one side through the file's git
    /// history; a direction with no further changes is disabled.
    fn render_version_steps(&self, side: DiffSide, cx: &mut Context<Self>) -> Div {
        let step = |step: Step, icon: IconName, cx: &mut Context<Self>| {
            let available =
                self.step_target(side, step).is_some() && self.step_blocked(side).is_none();
            let id = match (side, step) {
                (DiffSide::Left, Step::Older) => "left-older",
                (DiffSide::Left, Step::Newer) => "left-newer",
                (DiffSide::Right, Step::Older) => "right-older",
                (DiffSide::Right, Step::Newer) => "right-newer",
            };
            Segment::new(
                id,
                icon,
                ButtonState::enabled(available),
                button::Hint::text(self.step_hint(side, step)),
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    window.focus(&this.focus);
                    this.step_version(side, step, cx);
                }),
            )
        };
        button::segmented(vec![
            step(Step::Older, IconName::ArrowLeft, cx),
            step(Step::Newer, IconName::ArrowRight, cx),
        ])
    }

    fn render_banner(&self) -> Option<Div> {
        let blank = |side: DiffSide| {
            let buffer = &self.side(side).buffer;
            buffer.len() == 1 && buffer.line(0).is_empty()
        };
        if blank(DiffSide::Left) && blank(DiffSide::Right) {
            Some(file_header::banner(
                "Type or paste text on each side to compare it · ⌘⇧B beautifies JSON, dicts, and \\n",
                theme::subtext(),
            ))
        } else if self.identical {
            Some(file_header::banner("Files are identical", theme::green()))
        } else if !self.diff.has_changes() {
            Some(file_header::banner(
                "Files differ only in ignored content (whitespace, case, or line endings)",
                theme::yellow(),
            ))
        } else {
            None
        }
    }
}

/// Registers the editing commands on the element that holds the view.
fn bind_editing_actions(element: Div, cx: &mut Context<TextDiffView>) -> Div {
    macro_rules! on {
        ($element:expr, $action:ty, |$this:ident, $cx:ident| $body:expr) => {
            $element.on_action(cx.listener(|$this, _: &$action, _, $cx| $body))
        };
    }
    let element = on!(element, MoveLeft, |this, cx| this.move_cursor(Motion::Left, false, cx));
    let element = on!(element, MoveRight, |this, cx| this.move_cursor(Motion::Right, false, cx));
    let element = on!(element, MoveUp, |this, cx| this.move_cursor(Motion::Up, false, cx));
    let element = on!(element, MoveDown, |this, cx| this.move_cursor(Motion::Down, false, cx));
    let element =
        on!(element, MoveWordLeft, |this, cx| this.move_cursor(Motion::WordLeft, false, cx));
    let element =
        on!(element, MoveWordRight, |this, cx| this.move_cursor(Motion::WordRight, false, cx));
    let element =
        on!(element, MoveLineStart, |this, cx| this.move_cursor(Motion::LineStart, false, cx));
    let element =
        on!(element, MoveLineEnd, |this, cx| this.move_cursor(Motion::LineEnd, false, cx));
    let element =
        on!(element, MoveDocStart, |this, cx| this.move_cursor(Motion::DocStart, false, cx));
    let element = on!(element, MoveDocEnd, |this, cx| this.move_cursor(Motion::DocEnd, false, cx));
    let element = on!(element, MovePageUp, |this, cx| this.move_cursor(Motion::PageUp, false, cx));
    let element =
        on!(element, MovePageDown, |this, cx| this.move_cursor(Motion::PageDown, false, cx));
    let element = on!(element, SelectLeft, |this, cx| this.move_cursor(Motion::Left, true, cx));
    let element = on!(element, SelectRight, |this, cx| this.move_cursor(Motion::Right, true, cx));
    let element = on!(element, SelectUp, |this, cx| this.move_cursor(Motion::Up, true, cx));
    let element = on!(element, SelectDown, |this, cx| this.move_cursor(Motion::Down, true, cx));
    let element =
        on!(element, SelectWordLeft, |this, cx| this.move_cursor(Motion::WordLeft, true, cx));
    let element =
        on!(element, SelectWordRight, |this, cx| this.move_cursor(Motion::WordRight, true, cx));
    let element =
        on!(element, SelectLineStart, |this, cx| this.move_cursor(Motion::LineStart, true, cx));
    let element =
        on!(element, SelectLineEnd, |this, cx| this.move_cursor(Motion::LineEnd, true, cx));
    let element =
        on!(element, SelectDocStart, |this, cx| this.move_cursor(Motion::DocStart, true, cx));
    let element = on!(element, SelectDocEnd, |this, cx| this.move_cursor(Motion::DocEnd, true, cx));
    let element = on!(element, SelectPageUp, |this, cx| this.move_cursor(Motion::PageUp, true, cx));
    let element =
        on!(element, SelectPageDown, |this, cx| this.move_cursor(Motion::PageDown, true, cx));
    let element = on!(element, SelectAll, |this, cx| this.select_all(cx));
    let element =
        on!(element, Backspace, |this, cx| this.delete(motion::prev_char, EditKind::Backspace, cx));
    let element = on!(element, DeleteForward, |this, cx| this.delete(
        motion::next_char,
        EditKind::DeleteForward,
        cx
    ));
    let element = on!(element, DeleteWordLeft, |this, cx| this.delete(
        motion::prev_word,
        EditKind::Backspace,
        cx
    ));
    let element = on!(element, DeleteWordRight, |this, cx| this.delete(
        motion::next_word,
        EditKind::DeleteForward,
        cx
    ));
    let element = on!(element, DeleteToLineStart, |this, cx| this.delete(
        line_start_or_prev_char,
        EditKind::Backspace,
        cx
    ));
    let element = on!(element, Newline, |this, cx| this.newline(cx));
    let element = on!(element, Indent, |this, cx| this.indent(cx));
    let element = on!(element, Copy, |this, cx| this.copy(cx));
    let element = on!(element, Cut, |this, cx| this.cut(cx));
    let element = on!(element, Paste, |this, cx| this.paste(cx));
    let element = on!(element, Undo, |this, cx| this.undo(cx));
    let element = on!(element, Redo, |this, cx| this.redo(cx));
    let element = on!(element, CollapseSelection, |this, cx| this.collapse_selection(cx));
    let element = on!(element, CopyBlockToRight, |this, cx| {
        this.copy_target_section(DiffSide::Right, cx)
    });
    let element =
        on!(element, CopyBlockToLeft, |this, cx| { this.copy_target_section(DiffSide::Left, cx) });
    let element =
        on!(element, CopyLineToRight, |this, cx| this.copy_target_line(DiffSide::Right, cx));
    let element =
        on!(element, CopyLineToLeft, |this, cx| this.copy_target_line(DiffSide::Left, cx));
    let element = on!(element, InsertLine, |this, cx| this.insert_line(cx));
    element.on_action(cx.listener(|_, _: &ShowCharacterPalette, window, _| {
        window.show_character_palette();
    }))
}

/// The editing commands at the top of the context menu, enabled for the
/// row that was right-clicked when the menu opens.
fn edit_menu_items(view: WeakEntity<TextDiffView>) -> MenuItems {
    Rc::new(move |menu, _, cx| {
        let state = view.upgrade().map(|view| view.read(cx).edit_menu_state()).unwrap_or_default();
        menu.menu_with_disabled("Insert Line", Box::new(InsertLine), !state.insert_line)
            .menu_with_disabled("Copy Line Left", Box::new(CopyLineToLeft), !state.copy_left)
            .menu_with_disabled("Copy Line Right", Box::new(CopyLineToRight), !state.copy_right)
            .menu_with_disabled("Copy Section Left", Box::new(CopyBlockToLeft), !state.copy_left)
            .menu_with_disabled("Copy Section Right", Box::new(CopyBlockToRight), !state.copy_right)
    })
}

/// Target of ⌘⌫: the start of the line, or the previous line's end when
/// already at the start.
fn line_start_or_prev_char(
    buffer: &compare_core::EditBuffer,
    at: compare_core::Position,
) -> compare_core::Position {
    if at.column == 0 {
        motion::prev_char(buffer, at)
    } else {
        compare_core::Position::new(at.line, 0)
    }
}

impl Render for TextDiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Lines re-register their layouts as they are painted.
        self.layouts.borrow_mut().clear();
        self.ensure_blame(cx);
        let input_handler = {
            let focus = self.focus.clone();
            let entity = cx.entity();
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                },
            )
            .size_0()
        };
        let rows = list(
            self.list.clone(),
            cx.processor(|this, index: usize, window, cx| this.render_row(index, window, cx)),
        )
        .flex_1()
        .bg(theme::base());

        let content = div().flex().flex_col().size_full().child(rows).child(input_handler);
        let menu = path_actions::with_edit_and_file_menu(
            content,
            &self.focus,
            self.sides[0].path.clone(),
            self.sides[1].path.clone(),
            edit_menu_items(cx.entity().downgrade()),
        );
        // Editing keys apply only while the text has focus, not the find bar.
        let editor = div()
            .flex_1()
            .min_h_0()
            .key_context(DIFF_EDITOR_CONTEXT)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.mouse_down_below_text(event, window, cx);
                }),
            )
            .child(menu);
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_headers(cx))
            .children(self.render_find_bar(cx))
            .children(self.render_banner())
            .child(editor)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.mouse_up()),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.mouse_up()),
            )
            .on_action(cx.listener(|this, _: &OpenFind, window, cx| {
                this.open_find(false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenReplace, window, cx| {
                this.open_find(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.find_step(true, cx)))
            .on_action(cx.listener(|this, _: &FindPrev, _, cx| this.find_step(false, cx)))
            .on_action(cx.listener(|this, _: &ReplaceNext, _, cx| this.replace_next(cx)))
            .on_action(cx.listener(|this, _: &ReplaceAll, _, cx| this.replace_all(cx)))
            .on_action(cx.listener(|this, _: &CloseFind, window, cx| this.close_find(window, cx)));
        bind_editing_actions(root, cx)
    }
}
