//! Undo and redo for an [`EditBuffer`](crate::EditBuffer).

use std::time::{Duration, Instant};

use crate::edit_buffer::Position;

/// Typing pauses longer than this start a new undo step.
const COALESCE_WINDOW: Duration = Duration::from_secs(1);

/// One replacement: `old_text` at `start` became `new_text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: Position,
    pub old_text: String,
    pub new_text: String,
}

impl Edit {
    /// The range `new_text` occupies after the edit.
    pub fn new_range(&self) -> std::ops::Range<Position> {
        self.start..self.start.after_insert(&self.new_text)
    }

    /// The range `old_text` occupied before the edit.
    pub fn old_range(&self) -> std::ops::Range<Position> {
        self.start..self.start.after_insert(&self.old_text)
    }
}

/// What produced an edit; consecutive typing or deleting merges into one
/// undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    /// Deleting backwards (backspace).
    Backspace,
    /// Deleting forwards (delete key).
    DeleteForward,
    /// Anything else: paste, cut, replace, copy from the other side, ...
    Other,
}

/// One undo step: edits applied in order, plus the selection around them.
#[derive(Clone, Debug)]
pub struct Transaction<S> {
    pub edits: Vec<Edit>,
    pub selection_before: S,
    pub selection_after: S,
    id: u64,
    kind: EditKind,
    last_change: Instant,
}

/// Undo and redo stacks, and the step the file was last saved at.
#[derive(Clone, Debug)]
pub struct History<S> {
    undo: Vec<Transaction<S>>,
    redo: Vec<Transaction<S>>,
    next_id: u64,
    /// Id of the top undo step when last saved; 0 means the empty stack.
    saved_id: Option<u64>,
    /// Set to stop the next edit merging into the previous step.
    break_next: bool,
}

impl<S> Default for History<S> {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 1,
            saved_id: Some(0),
            break_next: false,
        }
    }
}

impl<S: Clone> History<S> {
    /// Records an edit that has just been applied, merging it into the
    /// previous step when it continues the same typing or deleting.
    pub fn record(&mut self, edit: Edit, kind: EditKind, before: S, after: S, now: Instant) {
        self.redo.clear();
        if !std::mem::take(&mut self.break_next) && self.try_coalesce(&edit, kind, &after, now) {
            return;
        }
        self.push(vec![edit], kind, before, after, now);
    }

    /// Records several edits made by one command as a single undo step.
    pub fn record_group(&mut self, edits: Vec<Edit>, before: S, after: S, now: Instant) {
        if edits.is_empty() {
            return;
        }
        self.redo.clear();
        self.break_next = false;
        self.push(edits, EditKind::Other, before, after, now);
    }

    /// Makes the next edit start a new undo step (for example after the
    /// cursor was moved).
    pub fn break_coalescing(&mut self) {
        self.break_next = true;
    }

    /// Takes the step to undo; the caller reverts its edits in reverse order.
    pub fn undo(&mut self) -> Option<Transaction<S>> {
        let transaction = self.undo.pop()?;
        self.redo.push(transaction.clone());
        self.break_next = true;
        Some(transaction)
    }

    /// Takes the step to redo; the caller re-applies its edits in order.
    pub fn redo(&mut self) -> Option<Transaction<S>> {
        let transaction = self.redo.pop()?;
        self.undo.push(transaction.clone());
        self.break_next = true;
        Some(transaction)
    }

    /// Marks the current state as saved.
    pub fn mark_saved(&mut self) {
        self.saved_id = Some(self.current_id());
        self.break_next = true;
    }

    /// Forgets all steps and treats the current state as saved, for example
    /// after reloading the file.
    pub fn reset(&mut self) {
        *self = Self { next_id: self.next_id, ..Self::default() };
    }

    /// Returns true if the text differs from the last saved state.
    pub fn is_modified(&self) -> bool {
        self.saved_id != Some(self.current_id())
    }

    fn current_id(&self) -> u64 {
        self.undo.last().map_or(0, |t| t.id)
    }

    fn push(&mut self, edits: Vec<Edit>, kind: EditKind, before: S, after: S, now: Instant) {
        let id = self.next_id;
        self.next_id += 1;
        self.undo.push(Transaction {
            edits,
            selection_before: before,
            selection_after: after,
            id,
            kind,
            last_change: now,
        });
    }

    fn try_coalesce(&mut self, edit: &Edit, kind: EditKind, after: &S, now: Instant) -> bool {
        let saved_id = self.saved_id;
        let Some(last) = self.undo.last_mut() else {
            return false;
        };
        if last.kind != kind
            || kind == EditKind::Other
            || last.edits.len() != 1
            || Some(last.id) == saved_id
            || now.duration_since(last.last_change) > COALESCE_WINDOW
        {
            return false;
        }
        let previous = &mut last.edits[0];
        let merged = match kind {
            EditKind::Typing => {
                let continues = edit.old_text.is_empty()
                    && !edit.new_text.contains('\n')
                    && edit.start == previous.new_range().end;
                if continues {
                    previous.new_text.push_str(&edit.new_text);
                }
                continues
            }
            EditKind::Backspace => {
                let continues = edit.new_text.is_empty()
                    && previous.new_text.is_empty()
                    && edit.old_range().end == previous.start;
                if continues {
                    previous.start = edit.start;
                    previous.old_text.insert_str(0, &edit.old_text);
                }
                continues
            }
            EditKind::DeleteForward => {
                let continues = edit.new_text.is_empty()
                    && previous.new_text.is_empty()
                    && edit.start == previous.start;
                if continues {
                    previous.old_text.push_str(&edit.old_text);
                }
                continues
            }
            EditKind::Other => false,
        };
        if merged {
            last.selection_after = after.clone();
            last.last_change = now;
        }
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(column: usize, text: &str) -> Edit {
        Edit { start: Position::new(0, column), old_text: String::new(), new_text: text.into() }
    }

    #[test]
    fn consecutive_typing_is_one_step() {
        let mut history: History<()> = History::default();
        let now = Instant::now();
        history.record(typed(0, "a"), EditKind::Typing, (), (), now);
        history.record(typed(1, "b"), EditKind::Typing, (), (), now);
        assert!(history.is_modified());
        let step = history.undo().expect("one step");
        assert_eq!(step.edits, [typed(0, "ab")]);
        assert!(!history.is_modified());
        assert!(history.undo().is_none());
    }

    #[test]
    fn saving_starts_a_new_step_and_tracks_modification() {
        let mut history: History<()> = History::default();
        let now = Instant::now();
        history.record(typed(0, "a"), EditKind::Typing, (), (), now);
        history.mark_saved();
        assert!(!history.is_modified());
        history.record(typed(1, "b"), EditKind::Typing, (), (), now);
        assert!(history.is_modified());
        history.undo();
        assert!(!history.is_modified());
        history.redo();
        assert!(history.is_modified());
    }

    #[test]
    fn backspaces_merge_backwards() {
        let mut history: History<()> = History::default();
        let now = Instant::now();
        let delete = |column: usize, text: &str| Edit {
            start: Position::new(0, column),
            old_text: text.into(),
            new_text: String::new(),
        };
        history.record(delete(4, "e"), EditKind::Backspace, (), (), now);
        history.record(delete(3, "d"), EditKind::Backspace, (), (), now);
        assert_eq!(history.undo().expect("step").edits, [delete(3, "de")]);
    }

    #[test]
    fn a_pause_starts_a_new_step() {
        let mut history: History<()> = History::default();
        let now = Instant::now();
        history.record(typed(0, "a"), EditKind::Typing, (), (), now);
        history.record(typed(1, "b"), EditKind::Typing, (), (), now + Duration::from_secs(5));
        assert_eq!(history.undo().expect("step").edits, [typed(1, "b")]);
    }
}
