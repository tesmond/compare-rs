//! Saving and reverting each side, and tracking unsaved changes.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui::{Context, EventEmitter, PromptLevel, Task, Window};

use compare_core::{DiffSide, EditBuffer, FileInfo, LiveDiff, TextDocument};

use super::{TextDiffView, read_only_reason};
use crate::crash_log;
use crate::document_syntax::DocumentSyntax;

/// Events a text comparison reports to its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextDiffEvent {
    /// A side was written to this path.
    Saved(PathBuf),
}

impl EventEmitter<TextDiffEvent> for TextDiffView {}

impl TextDiffView {
    /// Returns true if either side has unsaved changes.
    pub fn is_modified(&self) -> bool {
        self.sides.iter().any(|state| state.is_modified())
    }

    /// Names of the sides with unsaved changes, for prompts.
    pub fn modified_names(&self) -> Vec<String> {
        self.sides
            .iter()
            .filter(|state| state.is_modified())
            .map(|state| state.display_name())
            .collect()
    }

    /// Saves the side with the cursor.
    pub fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        self.save(self.active, window, cx)
    }

    /// Saves every side with unsaved changes, one after another. Resolves to
    /// false if any save was cancelled or failed.
    pub fn save_all(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let sides: Vec<DiffSide> = [DiffSide::Left, DiffSide::Right]
            .into_iter()
            .filter(|&side| self.side(side).is_modified())
            .collect();
        cx.spawn_in(window, async move |this, cx| {
            for side in sides {
                let Ok(save) = this.update_in(cx, |this, window, cx| this.save(side, window, cx))
                else {
                    return false;
                };
                if !save.await {
                    return false;
                }
            }
            true
        })
    }

    /// Writes `side` to its file, asking for a location if it has none and
    /// for confirmation if the file changed on disk since it was loaded.
    /// Resolves to true once saved.
    pub fn save(
        &mut self,
        side: DiffSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let state = self.side(side);
        if state.read_only.is_some() {
            return Task::ready(false);
        }
        let name = state.display_name();
        let path = state.path.clone();
        let changed_on_disk = match (&path, state.disk_modified) {
            (Some(path), Some(loaded)) => disk_modified(path).is_some_and(|now| now != loaded),
            _ => false,
        };
        let overwrite = changed_on_disk.then(|| {
            window.prompt(
                PromptLevel::Warning,
                &format!("{name} changed on disk since it was opened."),
                Some("Saving will replace the changes made outside compare-rs."),
                &["Overwrite", "Cancel"],
                cx,
            )
        });
        let choose_path = path.is_none().then(|| {
            let other = &self.side(side.other()).path;
            let folder = other
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("/"));
            cx.prompt_for_new_path(&folder, Some("Untitled.txt"))
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Some(answer) = overwrite
                && answer.await != Ok(0)
            {
                return false;
            }
            let path = match (path, choose_path) {
                (Some(path), _) => path,
                (None, Some(chosen)) => match chosen.await {
                    Ok(Ok(Some(path))) => path,
                    _ => return false,
                },
                (None, None) => return false,
            };
            let Ok((bytes, version)) = this.update(cx, |this, _| {
                let state = this.side_mut(side);
                (state.buffer.to_bytes(), state.history.begin_save())
            }) else {
                return false;
            };
            let written = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move { compare_core::write_atomically(&path, &bytes) }
                })
                .await;
            this.update_in(cx, |this, window, cx| match written {
                Ok(info) => {
                    this.finish_save(side, path, info, version, cx);
                    true
                }
                Err(error) => {
                    crash_log::record_error("Saving a file", &error.to_string());
                    let shown = window.prompt(
                        PromptLevel::Critical,
                        &format!("Could not save {}.", path.display()),
                        Some(&error.to_string()),
                        &["OK"],
                        cx,
                    );
                    cx.spawn(async move |_, _| {
                        shown.await.ok();
                    })
                    .detach();
                    false
                }
            })
            .unwrap_or(false)
        })
    }

    fn finish_save(
        &mut self,
        side: DiffSide,
        path: PathBuf,
        info: FileInfo,
        version: u64,
        cx: &mut Context<Self>,
    ) {
        let state = self.side_mut(side);
        state.history.mark_saved_version(version);
        if state.path.as_ref() != Some(&path) {
            // Newly named text: highlight it for its file type.
            state.syntax = DocumentSyntax::from_lines(&path, state.buffer.lines());
        }
        state.path = Some(path.clone());
        state.disk_modified = info.modified;
        state.info = info;
        cx.emit(TextDiffEvent::Saved(path));
        cx.notify();
    }

    /// Discards unsaved changes to the side with the cursor by reloading it.
    pub fn revert_active(&mut self, cx: &mut Context<Self>) {
        self.revert(self.active, cx);
    }

    /// Reloads `side` from its file, discarding unsaved changes.
    pub fn revert(&mut self, side: DiffSide, cx: &mut Context<Self>) {
        let Some(path) = self.side(side).path.clone() else {
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                crash_log::record_error("Reverting a file", &error.to_string());
                return;
            }
        };
        let document = TextDocument::decode(&bytes);
        let state = self.side_mut(side);
        state.buffer = EditBuffer::from_document(&document);
        state.syntax = DocumentSyntax::new(&path, &document);
        state.syntax.cover(state.buffer.lines());
        state.history.reset();
        state.read_only = read_only_reason(&document);
        state.info = FileInfo::from_path(&path);
        state.disk_modified = state.info.modified;
        state.marked = None;
        let head = state.buffer.clamp(state.selection.head);
        state.selection = super::side::Selection::caret(head);

        let [left, right] = &self.sides;
        self.diff = LiveDiff::compute(left.buffer.lines(), right.buffer.lines(), &self.options);
        self.identical = left.buffer.lines() == right.buffer.lines();
        self.list.reset(self.diff.rows().len());
        self.current_change = None;
        self.revision += 1;
        self.update_matches(false, cx);
        cx.notify();
    }
}

/// The file's current modification time, if it can be read.
fn disk_modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}
