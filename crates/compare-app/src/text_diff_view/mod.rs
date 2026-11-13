//! Side-by-side view of a text comparison, with both sides editable.
//!
//! Each side is an [`EditBuffer`]. Edits re-align the changed region of the
//! diff immediately ([`LiveDiff::apply_edit`]); a full diff then runs in the
//! background once typing pauses, in case a better alignment exists.

mod copying;
mod editing;
mod find;
mod git_versions;
mod input_handler;
mod motion;
mod render;
mod saving;
mod side;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Context, FocusHandle, Focusable, ListAlignment, ListOffset, ListState, SharedString, Task,
    TextLayout, px,
};

use compare_core::{
    DiffOptions, DiffSide, EditBuffer, Encoding, FileInfo, LiveDiff, Position, RowKind, TextDiff,
    TextDocument,
};

use crate::document_syntax::DocumentSyntax;
use crate::format;
use crate::horizontal_scroll::HorizontalScroll;

pub use find::{FIND_QUERY_CONTEXT, REPLACE_QUERY_CONTEXT};
pub use saving::TextDiffEvent;
use side::SideState;

/// How long typing must pause before the whole comparison is re-diffed.
const REDIFF_DELAY: Duration = Duration::from_millis(150);

/// Text layouts of the lines painted in the last frame, by side and line.
type LineLayouts = Rc<RefCell<HashMap<(DiffSide, usize), TextLayout>>>;

/// Shows two texts side by side with changed lines and words highlighted,
/// and lets either side be edited.
pub struct TextDiffView {
    focus: FocusHandle,
    sides: [SideState; 2],
    diff: LiveDiff,
    options: DiffOptions,
    list: ListState,
    horizontal: HorizontalScroll,
    /// Index into the diff's change starts of the change last navigated to.
    current_change: Option<usize>,
    /// Where navigating to that change left the cursor. While the cursor is
    /// still there, Next and Previous step from that change; once it has
    /// moved, they step from the cursor instead.
    change_cursor: Option<(DiffSide, Position)>,
    /// A problem to show above the text, such as a file that could not be
    /// reloaded.
    notice: Option<SharedString>,
    /// What each recently drawn row shows, kept until the text or diff
    /// changes.
    row_cache: render::RowCache,
    /// False for pasted text, whose "paths" are only labels.
    file_backed: bool,
    wrap: bool,
    /// True if both sides hold exactly the same lines.
    identical: bool,
    /// The side with the cursor.
    active: DiffSide,
    layouts: LineLayouts,
    /// True while the mouse button is held after a click in the text.
    dragging: bool,
    /// The pending full re-diff.
    rediff: Option<Task<()>>,
    /// Counts edits, so a re-diff of older text is discarded.
    revision: u64,
    /// The find bar, when open.
    find: Option<find::FindState>,
    /// The side and row last right-clicked, which context-menu commands act
    /// on (it may be a blank spacer row, which the cursor cannot be on).
    context_row: Option<(DiffSide, usize)>,
    /// The git history being browsed, if this comparison shows one.
    git: Option<git_versions::GitState>,
}

impl TextDiffView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        left: FileInfo,
        right: FileInfo,
        identical: bool,
        diff: TextDiff,
        left_syntax: DocumentSyntax,
        right_syntax: DocumentSyntax,
        options: DiffOptions,
        cx: &mut Context<Self>,
    ) -> Self {
        let left_buffer = EditBuffer::from_document(&diff.left);
        let right_buffer = EditBuffer::from_document(&diff.right);
        let live = LiveDiff::from_text_diff(&diff, left_buffer.len(), right_buffer.len());
        let max_columns = max_display_columns(&left_buffer).max(max_display_columns(&right_buffer));
        let mut sides = [
            SideState::new(left, left_buffer, left_syntax),
            SideState::new(right, right_buffer, right_syntax),
        ];
        for (state, document) in sides.iter_mut().zip([&diff.left, &diff.right]) {
            state.read_only = read_only_reason(document);
        }
        let list = ListState::new(live.rows().len(), ListAlignment::Top, px(300.0));
        Self {
            focus: cx.focus_handle(),
            sides,
            diff: live,
            options,
            list,
            horizontal: HorizontalScroll::new(max_columns),
            current_change: None,
            change_cursor: None,
            notice: None,
            row_cache: render::RowCache::default(),
            file_backed: true,
            wrap: false,
            identical,
            active: DiffSide::Left,
            layouts: LineLayouts::default(),
            dragging: false,
            rediff: None,
            revision: 0,
            find: None,
            context_row: None,
            git: None,
        }
    }

    /// Marks the sides as pasted text rather than files, which hides the
    /// copy-path and reveal-in-Finder menu items.
    pub fn without_file_paths(mut self) -> Self {
        self.file_backed = false;
        for state in &mut self.sides {
            state.path = None;
            state.disk_modified = None;
        }
        self
    }

    /// Makes sides that cannot be edited read-only, with the reason shown in
    /// their headers.
    pub fn with_read_only(mut self, reasons: [Option<&'static str>; 2]) -> Self {
        for (state, reason) in self.sides.iter_mut().zip(reasons) {
            if let Some(reason) = reason {
                state.read_only = Some(reason.into());
            }
        }
        self
    }

    /// Sets whether long lines wrap.
    pub fn with_wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Turns line wrapping on or off.
    pub fn set_wrap(&mut self, wrap: bool, cx: &mut Context<Self>) {
        if self.wrap == wrap {
            return;
        }
        self.wrap = wrap;
        self.horizontal.reset();
        // Row heights change, so every row must be measured again.
        let top = self.list.logical_scroll_top();
        self.list.reset(self.diff.rows().len());
        self.list.scroll_to(top);
        cx.notify();
    }

    /// Changes what counts as a difference and re-diffs the current text.
    pub fn set_options(&mut self, options: DiffOptions, cx: &mut Context<Self>) {
        if self.options != options {
            self.options = options;
            self.schedule_rediff(Duration::ZERO, cx);
        }
    }

    /// One-line description for the status bar.
    pub fn summary(&self) -> String {
        let lines = self.sides[0].buffer.len();
        if self.identical {
            return format!("Identical · {lines} lines");
        }
        let differences = match (self.diff.change_count(), self.current_change) {
            (0, _) => "Only ignored differences (whitespace, case, or line endings)".to_owned(),
            (count, Some(current)) if current < count => {
                format!("Difference {} of {count}", current + 1)
            }
            (1, _) => "1 difference".to_owned(),
            (count, _) => format!("{count} differences"),
        };
        match self.sides[0].syntax.language_name() {
            Some(language) => format!("{differences} · {language}"),
            None => differences,
        }
    }

    /// Returns true if any lines differ (under the current options).
    pub fn has_differences(&self) -> bool {
        self.diff.has_changes()
    }

    /// Moves to the next changed region, wrapping to the first.
    pub fn next_difference(&mut self, cx: &mut Context<Self>) {
        self.step_difference(true, cx);
    }

    /// Moves to the previous changed region, wrapping to the last.
    pub fn prev_difference(&mut self, cx: &mut Context<Self>) {
        self.step_difference(false, cx);
    }

    fn step_difference(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.diff.change_count();
        if count == 0 {
            return;
        }
        let head = self.sides[self.active.index()].selection.head;
        let current = self.current_change.filter(|&current| current < count);
        let stepping = current.filter(|_| self.change_cursor == Some((self.active, head)));
        let target = match stepping {
            Some(current) if forward => (current + 1) % count,
            Some(current) => (current + count - 1) % count,
            // The cursor has moved since the last step: go by where it is.
            None => {
                let row = self.diff.row_of_line(self.active, head.line);
                let at_or_after = self.diff.change_starts().partition_point(|&start| start < row);
                if forward {
                    // A change starting on the cursor's row counts as next,
                    // unless it is the one already marked.
                    let skip = usize::from(current == Some(at_or_after));
                    (at_or_after + skip) % count
                } else {
                    (at_or_after + count - 1) % count
                }
            }
        };
        self.go_to_change(target, cx);
    }

    /// Scrolls a change into view (with a little context above it) and puts
    /// the cursor at its start.
    fn go_to_change(&mut self, change: usize, cx: &mut Context<Self>) {
        self.current_change = Some(change);
        let row = self.diff.change_starts()[change];
        self.list.scroll_to(ListOffset { item_ix: row.saturating_sub(3), offset_in_item: px(0.0) });
        let line = self.diff.lines_before_row(self.active, row);
        let line = line.min(self.sides[self.active.index()].buffer.len() - 1);
        self.set_cursor(Position::new(line, 0));
        let head = self.sides[self.active.index()].selection.head;
        self.change_cursor = Some((self.active, head));
        cx.notify();
    }

    /// Row range of the current change, used to mark it in the gutter.
    fn current_change_rows(&self) -> Option<Range<usize>> {
        let start = *self.diff.change_starts().get(self.current_change?)?;
        let rows = self.diff.rows();
        let end = rows[start..]
            .iter()
            .position(|row| row.kind == RowKind::Equal)
            .map_or(rows.len(), |len| start + len);
        Some(start..end)
    }

    /// Re-diffs both texts in full after `delay`, unless they change again
    /// first.
    fn schedule_rediff(&mut self, delay: Duration, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
        self.rediff = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok((left, right, options)) = this.update(cx, |this, _| {
                (this.sides[0].buffer.joined(), this.sides[1].buffer.joined(), this.options)
            }) else {
                return;
            };
            let diff = cx
                .background_executor()
                .spawn(async move {
                    let left: Vec<&str> = left.split('\n').collect();
                    let right: Vec<&str> = right.split('\n').collect();
                    LiveDiff::compute(&left, &right, &options)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.revision == revision {
                    this.set_diff(diff, cx);
                }
            })
            .ok();
        }));
    }

    /// Shows a freshly computed diff, keeping the same lines in view.
    fn set_diff(&mut self, diff: LiveDiff, cx: &mut Context<Self>) {
        self.identical = self.sides[0].buffer.lines() == self.sides[1].buffer.lines();
        if diff == self.diff {
            cx.notify();
            return;
        }
        let top = self.list.logical_scroll_top();
        let anchor = self.diff.rows().get(top.item_ix).and_then(|row| {
            let left = row.left.map(|line| (DiffSide::Left, line));
            left.or(row.right.map(|line| (DiffSide::Right, line)))
        });
        self.diff = diff;
        self.invalidate_rows();
        // The changes may have been renumbered.
        self.current_change = None;
        let rows = self.diff.rows().len();
        self.list.reset(rows);
        let item_ix = anchor.map_or(top.item_ix.min(rows), |(side, line)| {
            self.diff.row_of_line(side, line as usize)
        });
        self.list.scroll_to(ListOffset { item_ix, offset_in_item: top.offset_in_item });
        cx.notify();
    }
}

impl Focusable for TextDiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Why a document cannot be edited, if it cannot.
fn read_only_reason(document: &TextDocument) -> Option<SharedString> {
    if !document.is_lossy() {
        return None;
    }
    Some(match document.encoding() {
        Encoding::Utf16Le | Encoding::Utf16Be => "Read-only: invalid UTF-16".into(),
        Encoding::Utf8 | Encoding::Utf8Bom => "Read-only: invalid UTF-8".into(),
    })
}

/// Widest line of `buffer`, in display columns.
fn max_display_columns(buffer: &EditBuffer) -> usize {
    buffer.lines().iter().map(|line| display_width(line)).max().unwrap_or(0)
}

/// Width of `line` in display columns (after tab expansion).
fn display_width(line: &str) -> usize {
    if line.contains('\t') {
        format::display_column(line, line.len())
    } else {
        line.chars().count()
    }
}
