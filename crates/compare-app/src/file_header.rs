//! Header bars that describe the files being compared.

use std::path::Path;

use gpui::{
    AnyElement, Div, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::tooltip::Tooltip;

use compare_core::FileInfo;

use crate::format;
use crate::theme;

/// The longest folder shown under a file name before its start is elided.
const MAX_FOLDER_CHARS: usize = 72;

/// A two-line header: the file name above its folder, with details (size,
/// date, notes) on the right. Callers may append buttons after the details.
///
/// The bar grows to fill the width of a row; in a column, wrap it in a row.
pub fn titled_header(
    name: impl Into<SharedString>,
    folder: impl Into<SharedString>,
    details: impl IntoElement,
) -> Div {
    bar()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .justify_center()
                .child(
                    div()
                        .truncate()
                        .text_size(theme::ui_text_size())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text())
                        .line_height(px(15.0))
                        .child(name.into()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(theme::meta_text_size())
                        .text_color(theme::muted())
                        .line_height(px(13.0))
                        .child(folder.into()),
                ),
        )
        .child(meta(details))
}

/// The file another one is compared with, for the size and date tooltips.
#[derive(Clone, Copy)]
pub struct Counterpart<'a> {
    pub info: &'a FileInfo,
    /// How the tooltips name it, e.g. "right file".
    pub name: &'static str,
}

/// A header naming a file, with its size and modification time. With a
/// counterpart, hovering the size or date says how they compare.
pub fn file_header(id: &'static str, info: &FileInfo, other: Option<Counterpart<'_>>) -> Div {
    titled_header(
        format::file_name(&info.path),
        folder_label(&info.path),
        file_details(id, info, other),
    )
}

/// A file's size and modification time. With a counterpart, hovering the
/// size says how much larger or smaller the file is, and hovering the date
/// how much newer or older.
pub fn file_details(id: &'static str, info: &FileInfo, other: Option<Counterpart<'_>>) -> Div {
    let size_hint =
        other.map(|other| format::size_difference(info.size, other.info.size, other.name));
    let date_hint =
        other.map(|other| format::time_difference(info.modified, other.info.modified, other.name));
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap_1()
        .child(hoverable(
            ElementId::Name(format!("{id}-size").into()),
            format::format_size(info.size),
            size_hint,
        ))
        .child("·")
        .child(hoverable(
            ElementId::Name(format!("{id}-date").into()),
            format::format_time(info.modified),
            date_hint,
        ))
}

/// A detail label that explains itself in a tooltip on hover.
fn hoverable(id: ElementId, label: String, hint: Option<String>) -> AnyElement {
    let Some(hint) = hint else {
        return label.into_any_element();
    };
    let hint = SharedString::from(hint);
    div()
        .id(id)
        .cursor_default()
        .hover(|style| style.text_color(theme::text()))
        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
        .child(label)
        .into_any_element()
}

/// The folder holding `path`, shortened from the start so its end stays
/// visible.
pub fn folder_label(path: &Path) -> String {
    format::elide_start(&format::parent_label(path), MAX_FOLDER_CHARS)
}

/// A single-line header bar with a primary label and secondary details on
/// the right, the same height as [`titled_header`].
pub fn header_bar(label: impl Into<SharedString>, details: impl Into<SharedString>) -> Div {
    let details: SharedString = details.into();
    bar()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(theme::ui_text_size())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text())
                .child(label.into()),
        )
        .child(meta(details))
}

/// Right-aligned secondary details of a header.
fn meta(details: impl IntoElement) -> Div {
    div().flex_none().text_size(theme::meta_text_size()).text_color(theme::muted()).child(details)
}

fn bar() -> Div {
    div()
        .flex()
        .flex_row()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap_2()
        .h(theme::header_height())
        .px_2()
        .bg(theme::mantle())
        .border_b_1()
        .border_color(theme::surface())
}

/// A full-width notice shown above a comparison.
pub fn banner(message: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .flex()
        .justify_center()
        .py(px(4.0))
        .bg(theme::crust())
        .border_b_1()
        .border_color(theme::surface())
        .text_size(theme::summary_text_size())
        .text_color(color)
        .child(message.into())
}
