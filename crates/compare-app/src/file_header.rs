//! Header bars that describe the files being compared.

use gpui::{Div, Hsla, ParentElement, SharedString, Styled, div, px};

use compare_core::FileInfo;

use crate::format;
use crate::theme;

/// A header showing a file's path, size, and modification time.
pub fn file_header(info: &FileInfo) -> Div {
    let details =
        format!("{} · {}", format::format_size(info.size), format::format_time(info.modified));
    header_bar(format::path_label(&info.path), details)
}

/// Like [`file_header`], with a note (such as "read-only") after the
/// details.
pub fn file_header_with_note(info: &FileInfo, note: Option<&str>) -> Div {
    let mut details =
        format!("{} · {}", format::format_size(info.size), format::format_time(info.modified));
    if let Some(note) = note {
        details = format!("{note} · {details}");
    }
    header_bar(format::path_label(&info.path), details)
}

/// A header bar with a primary label and secondary details on the right.
///
/// The bar grows to fill the width of a row; in a column, wrap it in a row.
pub fn header_bar(label: impl Into<SharedString>, details: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .justify_between()
        .gap_2()
        .h(px(24.0))
        .px_2()
        .bg(theme::mantle())
        .border_b_1()
        .border_color(theme::surface())
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(11.0))
                .text_color(theme::text())
                .child(label.into()),
        )
        .child(
            div().flex_none().text_size(px(10.0)).text_color(theme::muted()).child(details.into()),
        )
}

/// A full-width notice shown above a comparison.
pub fn banner(message: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .flex()
        .justify_center()
        .py(px(3.0))
        .bg(theme::crust())
        .border_b_1()
        .border_color(theme::surface())
        .text_size(px(11.0))
        .text_color(color)
        .child(message.into())
}
