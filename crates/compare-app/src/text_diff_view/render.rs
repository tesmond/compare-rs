//! Drawing the comparison: aligned rows of editable lines.

use std::ops::Range;

use gpui::{
    AnyElement, Context, CursorStyle, Div, ElementInputHandler, Hsla, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Render,
    ScrollWheelEvent, Styled, Window, canvas, div, list, px,
};

use compare_core::{DiffSide, RowKind};
use compare_diff::InlineDiff;

use super::TextDiffView;
use super::editing::Motion;
use crate::actions::{
    Backspace, CollapseSelection, Copy, Cut, DIFF_EDITOR_CONTEXT, DeleteForward, DeleteToLineStart,
    DeleteWordLeft, DeleteWordRight, Indent, MoveDocEnd, MoveDocStart, MoveDown, MoveLeft,
    MoveLineEnd, MoveLineStart, MovePageDown, MovePageUp, MoveRight, MoveUp, MoveWordLeft,
    MoveWordRight, Newline, Paste, Redo, SelectAll, SelectDocEnd, SelectDocStart, SelectDown,
    SelectLeft, SelectLineEnd, SelectLineStart, SelectPageDown, SelectPageUp, SelectRight,
    SelectUp, SelectWordLeft, SelectWordRight, ShowCharacterPalette, Undo,
};
use crate::code_text::{self, Mark};
use crate::editor_line::EditorLine;
use crate::file_header;
use crate::format;
use crate::path_actions;
use crate::theme;

use super::motion;
use compare_core::EditKind;

/// Width of the line-number gutter.
pub(super) const GUTTER_WIDTH: f32 = 52.0;

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
            .child(div().flex_none().w(px(3.0)).bg(marker))
            .child(left)
            .child(div().flex_none().w(px(1.0)).bg(theme::surface()))
            .child(right)
            .into_any_element()
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
                    this.mouse_down(side, row, event, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                this.mouse_drag(side, row, event, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(GUTTER_WIDTH))
                    .pr_2()
                    .flex()
                    .justify_end()
                    .text_color(theme::gutter())
                    .bg(theme::mantle())
                    .child(number),
            )
            .child(div().flex_1().min_w_0().pl_2().overflow_hidden().child(text))
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

    fn render_headers(&self) -> Div {
        let header = |side: DiffSide| {
            let state = self.side(side);
            file_header::file_header_with_note(
                &state.info,
                state.read_only.as_ref().map(|note| note.as_ref()),
            )
        };
        div()
            .flex()
            .flex_row()
            .child(header(DiffSide::Left))
            .child(div().w(px(1.0)).bg(theme::surface()))
            .child(header(DiffSide::Right))
    }

    fn render_banner(&self) -> Option<Div> {
        if self.identical {
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
    element.on_action(cx.listener(|_, _: &ShowCharacterPalette, window, _| {
        window.show_character_palette();
    }))
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

        let content = div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_headers())
            .children(self.render_banner())
            .child(rows)
            .child(input_handler);
        let file_backed = self.file_backed;
        let menu = path_actions::with_file_pair_menu(
            content,
            &self.focus,
            file_backed.then(|| self.sides[0].info.path.clone()),
            file_backed.then(|| self.sides[1].info.path.clone()),
        );
        let root = div()
            .size_full()
            .key_context(DIFF_EDITOR_CONTEXT)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.mouse_up()),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.mouse_up()),
            )
            .child(menu);
        bind_editing_actions(root, cx)
    }
}
