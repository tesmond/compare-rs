//! The state of one side of an editable comparison.

use std::ops::Range;
use std::path::PathBuf;
use std::time::SystemTime;

use gpui::SharedString;

use compare_core::{EditBuffer, FileInfo, History, Position};

use crate::document_syntax::DocumentSyntax;

/// A selection: the anchor stays put while the head (the cursor) moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Position,
    pub head: Position,
    /// Display column to aim for when moving up and down through shorter
    /// lines.
    pub goal_column: Option<usize>,
}

impl Selection {
    /// An empty selection (a plain cursor) at `position`.
    pub fn caret(position: Position) -> Self {
        Self { anchor: position, head: position, goal_column: None }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The selected range, start first.
    pub fn range(&self) -> Range<Position> {
        if self.anchor <= self.head { self.anchor..self.head } else { self.head..self.anchor }
    }
}

/// One file (or text) being compared and edited.
pub struct SideState {
    pub info: FileInfo,
    /// Where the text is saved; `None` for text that has never been saved.
    pub path: Option<PathBuf>,
    /// The file's modification time when it was loaded or last saved, to
    /// notice changes made by other programs.
    pub disk_modified: Option<SystemTime>,
    pub buffer: EditBuffer,
    pub syntax: DocumentSyntax,
    pub history: History<Selection>,
    pub selection: Selection,
    /// Why the side cannot be edited, if it cannot.
    pub read_only: Option<SharedString>,
    /// Text being composed by an input method, as a byte range of the
    /// cursor's line.
    pub marked: Option<Range<usize>>,
}

impl SideState {
    pub fn new(info: FileInfo, buffer: EditBuffer, mut syntax: DocumentSyntax) -> Self {
        syntax.cover(buffer.lines());
        Self {
            path: Some(info.path.clone()),
            disk_modified: info.modified,
            info,
            buffer,
            syntax,
            history: History::default(),
            selection: Selection::default(),
            read_only: None,
            marked: None,
        }
    }

    /// Returns true if the text differs from the last saved state.
    pub fn is_modified(&self) -> bool {
        self.history.is_modified()
    }

    /// The file name, or "Untitled" for text never saved.
    pub fn display_name(&self) -> String {
        match &self.path {
            Some(_) => self.info.display_name(),
            None => "Untitled".to_owned(),
        }
    }
}
