//! Side-by-side hex dump of a binary comparison.

use std::ops::Range;

use gpui::{
    Context, Div, FocusHandle, HighlightStyle, Hsla, IntoElement, ParentElement, Render,
    ScrollStrategy, SharedString, Styled, StyledText, UniformListScrollHandle, Window, div, px,
    uniform_list,
};

use compare_core::{BYTES_PER_ROW, BinaryDiff, FileInfo};

use crate::file_header::{self, Counterpart};
use crate::format;
use crate::path_actions;
use crate::theme;

/// Shows two binary files as hex dumps with differing bytes highlighted.
pub struct HexDiffView {
    focus: FocusHandle,
    left: FileInfo,
    right: FileInfo,
    identical: bool,
    diff: BinaryDiff,
    scroll: UniformListScrollHandle,
    /// Index into `diff.differing_rows` last navigated to.
    current: Option<usize>,
}

impl HexDiffView {
    pub fn new(
        left: FileInfo,
        right: FileInfo,
        identical: bool,
        diff: BinaryDiff,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus: cx.focus_handle(),
            left,
            right,
            identical,
            diff,
            scroll: UniformListScrollHandle::new(),
            current: None,
        }
    }

    /// One-line description for the status bar.
    pub fn summary(&self) -> String {
        if self.identical {
            return format!("Binary files are identical · {}", format::format_size(self.left.size));
        }
        format!(
            "Binary files differ · {} bytes different ({:.2}%) · {} vs {}",
            self.diff.differing_bytes,
            self.diff.difference_percent(),
            format::format_size(self.left.size),
            format::format_size(self.right.size),
        )
    }

    pub fn next_difference(&mut self, cx: &mut Context<Self>) {
        let count = self.diff.differing_rows.len();
        if count > 0 {
            let next = self.current.map_or(0, |current| (current + 1).min(count - 1));
            self.go_to(next, cx);
        }
    }

    pub fn prev_difference(&mut self, cx: &mut Context<Self>) {
        if !self.diff.differing_rows.is_empty() {
            let prev = self.current.map_or(0, |current| current.saturating_sub(1));
            self.go_to(prev, cx);
        }
    }

    fn go_to(&mut self, index: usize, cx: &mut Context<Self>) {
        self.current = Some(index);
        self.scroll.scroll_to_item(self.diff.differing_rows[index], ScrollStrategy::Center);
        cx.notify();
    }

    fn render_rows(&self, range: Range<usize>) -> Vec<Div> {
        let current_row = self.current.map(|i| self.diff.differing_rows[i]);
        range.map(|row| self.render_row(row, current_row == Some(row))).collect()
    }

    fn render_row(&self, row: usize, is_current: bool) -> Div {
        let start = row * BYTES_PER_ROW;
        let left = row_bytes(&self.diff.left, start);
        let right = row_bytes(&self.diff.right, start);
        let differs: Vec<bool> = (0..BYTES_PER_ROW).map(|i| left.get(i) != right.get(i)).collect();
        let marker = if is_current { theme::accent() } else { theme::base() };
        div()
            .flex()
            .flex_row()
            .w_full()
            .h(theme::row_height())
            .font_family(theme::MONO_FONT)
            .text_size(theme::code_text_size())
            .child(div().flex_none().w(px(3.0)).h_full().bg(marker))
            .child(hex_side(start, left, &differs, theme::deleted_emphasis()))
            .child(div().flex_none().w(px(1.0)).h_full().bg(theme::surface()))
            .child(hex_side(start, right, &differs, theme::inserted_emphasis()))
    }
}

fn row_bytes(data: &[u8], start: usize) -> &[u8] {
    if start >= data.len() { &[] } else { &data[start..(start + BYTES_PER_ROW).min(data.len())] }
}

/// Column of byte `index` within the hex text (an extra space after 8 bytes).
fn hex_column(index: usize) -> usize {
    index * 3 + usize::from(index >= BYTES_PER_ROW / 2)
}

fn hex_side(offset: usize, bytes: &[u8], differs: &[bool], emphasis: Hsla) -> Div {
    let mut hex = String::with_capacity(BYTES_PER_ROW * 3 + 1);
    let mut ascii = String::with_capacity(BYTES_PER_ROW);
    for index in 0..BYTES_PER_ROW {
        if index == BYTES_PER_ROW / 2 {
            hex.push(' ');
        }
        match bytes.get(index) {
            Some(byte) => {
                hex.push_str(&format!("{byte:02x} "));
                ascii.push(if byte.is_ascii_graphic() || *byte == b' ' {
                    *byte as char
                } else {
                    '.'
                });
            }
            None => {
                hex.push_str("   ");
                ascii.push(' ');
            }
        }
    }
    let style = HighlightStyle { background_color: Some(emphasis), ..Default::default() };
    let present = |index: &usize| *index < bytes.len() && differs[*index];
    let hex_highlights: Vec<(Range<usize>, HighlightStyle)> = (0..BYTES_PER_ROW)
        .filter(present)
        .map(|i| (hex_column(i)..hex_column(i) + 2, style))
        .collect();
    let ascii_highlights: Vec<(Range<usize>, HighlightStyle)> =
        (0..BYTES_PER_ROW).filter(present).map(|i| (i..i + 1, style)).collect();

    let background = if bytes.is_empty() { theme::filler_bg() } else { theme::base() };
    div()
        .flex()
        .flex_row()
        .flex_1()
        .min_w_0()
        .h_full()
        .gap_3()
        .px_2()
        .overflow_hidden()
        .whitespace_nowrap()
        .bg(background)
        .child(div().flex_none().text_color(theme::gutter()).child(format!("{offset:08x}")))
        .child(
            div()
                .flex_none()
                .text_color(theme::text())
                .child(StyledText::new(SharedString::from(hex)).with_highlights(hex_highlights)),
        )
        .child(
            div().flex_none().text_color(theme::subtext()).child(
                StyledText::new(SharedString::from(ascii)).with_highlights(ascii_highlights),
            ),
        )
}

impl Render for HexDiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let banner = if self.identical {
            file_header::banner("Binary files are identical", theme::green())
        } else {
            file_header::banner(self.summary(), theme::yellow())
        };
        let content = div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(file_header::file_header(
                        "hex-left",
                        &self.left,
                        Some(Counterpart { info: &self.right, name: "right file" }),
                    ))
                    .child(div().w(px(1.0)).bg(theme::surface()))
                    .child(file_header::file_header(
                        "hex-right",
                        &self.right,
                        Some(Counterpart { info: &self.left, name: "left file" }),
                    )),
            )
            .child(banner)
            .child(
                uniform_list(
                    "hex-diff-rows",
                    self.diff.row_count(),
                    cx.processor(|this, range: Range<usize>, _window, _cx| this.render_rows(range)),
                )
                .track_scroll(self.scroll.clone())
                .flex_1()
                .bg(theme::base()),
            );
        path_actions::with_file_pair_menu(
            content,
            &self.focus,
            Some(self.left.path.clone()),
            Some(self.right.path.clone()),
        )
    }
}
