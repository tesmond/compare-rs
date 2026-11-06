//! A comparison of text typed or pasted into the app, with no files involved.

use std::path::{Path, PathBuf};

use gpui::{
    AppContext, ClickEvent, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Task, Window, div, px,
};
use gpui_component::input::{Input, InputState};

use compare_core::{CompareOptions, FileInfo, TextDiff, TextDocument};

use crate::button::{self, ButtonState};
use crate::document_syntax::DocumentSyntax;
use crate::file_header;
use crate::text_diff_view::TextDiffView;
use crate::theme;

/// Labels shown in place of file paths.
const LEFT_LABEL: &str = "Left text";
const RIGHT_LABEL: &str = "Right text";

/// Two editors for pasted text, and the diff between them once compared.
pub struct PasteCompareView {
    left_editor: Entity<InputState>,
    right_editor: Entity<InputState>,
    /// The comparison being shown; `None` while editing.
    diff_view: Option<Entity<TextDiffView>>,
    options: CompareOptions,
    wrap: bool,
    /// Drops the result of an earlier, superseded comparison.
    pending: Option<Task<()>>,
}

impl PasteCompareView {
    pub fn new(options: CompareOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .code_editor("text")
                    .line_number(true)
                    .placeholder(placeholder)
            })
        };
        let left_editor = editor("Paste or type the left text…", window, cx);
        let right_editor = editor("Paste or type the right text…", window, cx);
        left_editor.update(cx, |state, cx| state.focus(window, cx));
        Self { left_editor, right_editor, diff_view: None, options, wrap: false, pending: None }
    }

    /// Returns true while the diff (rather than the editors) is shown.
    pub fn is_comparing(&self) -> bool {
        self.diff_view.is_some()
    }

    /// One-line description for the status bar.
    pub fn summary(&self, cx: &gpui::App) -> String {
        match &self.diff_view {
            Some(view) => view.read(cx).summary(),
            None if self.pending.is_some() => "Comparing…".to_owned(),
            None => "Paste text on each side, then press Compare (⌘↩)".to_owned(),
        }
    }

    /// Turns line wrapping on or off in the editors and the comparison.
    pub fn set_wrap(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.wrap = wrap;
        for editor in [&self.left_editor, &self.right_editor] {
            editor.update(cx, |state, cx| state.set_soft_wrap(wrap, window, cx));
        }
        if let Some(view) = &self.diff_view {
            view.update(cx, |view, cx| view.set_wrap(wrap, cx));
        }
    }

    /// Changes the options and re-runs the comparison if one is shown.
    pub fn set_options(&mut self, options: CompareOptions, cx: &mut Context<Self>) {
        self.options = options;
        if self.is_comparing() {
            self.compare(cx);
        }
    }

    /// Diffs the two texts on a background thread and shows the result.
    pub fn compare(&mut self, cx: &mut Context<Self>) {
        let left = self.left_editor.read(cx).value().to_string();
        let right = self.right_editor.read(cx).value().to_string();
        let options = self.options;
        let diff_options = options.diff;
        let wrap = self.wrap;
        let job = cx.background_executor().spawn(async move { diff_texts(left, right, &options) });
        self.pending = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                let PastedDiff { left, right, identical, diff, syntax } = result;
                let [left_syntax, right_syntax] = syntax;
                this.diff_view = Some(cx.new(|cx| {
                    TextDiffView::new(
                        left,
                        right,
                        identical,
                        diff,
                        left_syntax,
                        right_syntax,
                        diff_options,
                        cx,
                    )
                    .without_file_paths()
                    .with_wrap(wrap)
                }));
                this.pending = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Returns to the editors, keeping their text.
    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.diff_view = None;
        self.left_editor.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn swap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let left = self.left_editor.read(cx).value();
        let right = self.right_editor.read(cx).value();
        self.left_editor.update(cx, |state, cx| state.set_value(right, window, cx));
        self.right_editor.update(cx, |state, cx| state.set_value(left, window, cx));
        if self.is_comparing() {
            self.compare(cx);
        }
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for editor in [&self.left_editor, &self.right_editor] {
            editor.update(cx, |state, cx| state.set_value("", window, cx));
        }
        self.edit(window, cx);
    }

    pub fn next_difference(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.diff_view {
            view.update(cx, |view, cx| view.next_difference(cx));
        }
    }

    pub fn prev_difference(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.diff_view {
            view.update(cx, |view, cx| view.prev_difference(cx));
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mode_button = if self.is_comparing() {
            button::button(
                "paste-edit",
                "✎ Edit Text",
                ButtonState::Enabled,
                cx.listener(|this, _: &ClickEvent, window, cx| this.edit(window, cx)),
            )
        } else {
            button::button(
                "paste-compare",
                "▶ Compare",
                ButtonState::Enabled,
                cx.listener(|this, _: &ClickEvent, _, cx| this.compare(cx)),
            )
        };
        let hint = if self.options.beautify {
            "Beautify is on: JSON, Python dicts, and \\n escapes are expanded before comparing"
        } else {
            "Tip: turn on Beautify to expand JSON, Python dicts, and \\n escapes"
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
            .child(mode_button)
            .child(button::button(
                "paste-swap",
                "⇄ Swap Sides",
                ButtonState::Enabled,
                cx.listener(|this, _: &ClickEvent, window, cx| this.swap(window, cx)),
            ))
            .child(button::button(
                "paste-clear",
                "Clear",
                ButtonState::Enabled,
                cx.listener(|this, _: &ClickEvent, window, cx| this.clear(window, cx)),
            ))
            .child(div().flex_1())
            .child(div().text_size(px(10.0)).text_color(theme::muted()).child(hint))
    }
}

impl Render for PasteCompareView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.diff_view {
            Some(view) => div().flex_1().min_h_0().child(view.clone()),
            None => {
                let pane = |label: &'static str, editor: &Entity<InputState>| {
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().flex().flex_row().child(file_header::header_bar(
                            label,
                            SharedString::from("paste with ⌘V"),
                        )))
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .font_family(theme::MONO_FONT)
                                .text_size(theme::code_text_size())
                                .child(Input::new(editor).h_full().appearance(false)),
                        )
                };
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(pane(LEFT_LABEL, &self.left_editor))
                    .child(div().w(px(1.0)).bg(theme::surface()))
                    .child(pane(RIGHT_LABEL, &self.right_editor))
            }
        };
        div().flex().flex_col().size_full().child(self.render_toolbar(cx)).child(body)
    }
}

/// The background result of comparing pasted text.
struct PastedDiff {
    left: FileInfo,
    right: FileInfo,
    identical: bool,
    diff: TextDiff,
    syntax: [DocumentSyntax; 2],
}

fn diff_texts(left: String, right: String, options: &CompareOptions) -> PastedDiff {
    let identical = left == right;
    let info = |label: &str, text: &str| FileInfo {
        path: PathBuf::from(label),
        size: text.len() as u64,
        modified: None,
    };
    let (left_info, right_info) = (info(LEFT_LABEL, &left), info(RIGHT_LABEL, &right));
    let diff = TextDiff::with_options(TextDocument::new(left), TextDocument::new(right), options);
    let syntax = [guess_syntax(&diff.left), guess_syntax(&diff.right)];
    PastedDiff { left: left_info, right: right_info, identical, diff, syntax }
}

/// Picks a highlighter for text without a file name: bracketed data is shown
/// as JSON (or Python when it uses single quotes); anything else is plain.
fn guess_syntax(document: &TextDocument) -> DocumentSyntax {
    let text = document.text().trim_start();
    let pseudo_name = match text.chars().next() {
        Some('{' | '[') if text.contains('\'') => "pasted.py",
        Some('{' | '[') => "pasted.json",
        _ => return DocumentSyntax::default(),
    };
    DocumentSyntax::new(Path::new(pseudo_name), document)
}

#[cfg(test)]
mod tests {
    use compare_core::RowKind;

    use super::*;

    #[test]
    fn compares_pasted_json_after_beautifying() {
        let options = CompareOptions { beautify: true, ..Default::default() };
        let result = diff_texts(
            r#"{"name": "app", "port": 8080, "tags": ["a", "b"]}"#.to_owned(),
            r#"{"name": "app", "port": 9090, "tags": ["a", "c"]}"#.to_owned(),
            &options,
        );
        assert!(result.diff.beautified);
        assert!(!result.identical);
        let changed: Vec<usize> = result
            .diff
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.kind == RowKind::Changed)
            .map(|(index, _)| index)
            .collect();
        // Only the port line and the second tag differ.
        assert_eq!(changed, [2, 5]);
        assert_eq!(result.syntax[0].language_name(), Some("JSON"));
    }

    #[test]
    fn expands_escaped_newlines_in_console_output() {
        let options = CompareOptions { beautify: true, ..Default::default() };
        let result = diff_texts(
            r"step 1\nstep 2\nstep 3".to_owned(),
            "step 1\nstep 2\nstep 4".to_owned(),
            &options,
        );
        assert_eq!(result.diff.left.len(), 3);
        assert_eq!(result.diff.change_count(), 1);
    }

    #[test]
    fn without_beautify_compact_text_is_one_line() {
        let result = diff_texts(
            r#"{"a": 1}"#.to_owned(),
            r#"{"a": 2}"#.to_owned(),
            &CompareOptions::default(),
        );
        assert!(!result.diff.beautified);
        assert_eq!(result.diff.rows.len(), 1);
    }

    #[test]
    fn python_dicts_use_python_highlighting() {
        let document = TextDocument::new("{'a': None}".to_owned());
        assert_eq!(guess_syntax(&document).language_name(), Some("Python"));
        assert_eq!(guess_syntax(&TextDocument::new("plain".to_owned())).language_name(), None);
    }
}
