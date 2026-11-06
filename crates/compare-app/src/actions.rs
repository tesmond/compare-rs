//! Application-wide actions and their key bindings.

use gpui::{App, KeyBinding, actions};

actions!(
    compare_app,
    [
        NewTab,
        NewTextCompare,
        OpenGitHistory,
        OpenLeft,
        OpenRight,
        OpenFolderLeft,
        OpenFolderRight,
        Compare,
        CompareFolders,
        Merge,
        CloseTab,
        SaveFile,
        SaveAll,
        RevertFile,
        NextTab,
        PrevTab,
        NextDifference,
        PrevDifference,
        ToggleIgnoreWhitespace,
        ToggleIgnoreCase,
        Beautify,
        ToggleWordWrap,
        CopyPaths,
        Quit,
    ]
);

actions!(folder_view, [SelectNext, SelectPrev, Expand, Collapse, OpenSelected]);

// Context-menu actions, handled by whichever comparison view was right-clicked.
actions!(context_menu, [CopyLeftPath, CopyRightPath, RevealLeft, RevealRight]);

// Text editing in a side-by-side comparison.
actions!(
    editor,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        MoveWordLeft,
        MoveWordRight,
        MoveLineStart,
        MoveLineEnd,
        MoveDocStart,
        MoveDocEnd,
        MovePageUp,
        MovePageDown,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectLineStart,
        SelectLineEnd,
        SelectDocStart,
        SelectDocEnd,
        SelectPageUp,
        SelectPageDown,
        SelectAll,
        Backspace,
        DeleteForward,
        DeleteWordLeft,
        DeleteWordRight,
        DeleteToLineStart,
        Newline,
        Indent,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        CollapseSelection,
        ShowCharacterPalette,
        CopyBlockToRight,
        CopyBlockToLeft,
        CopyLineToRight,
        CopyLineToLeft,
        InsertLine,
        OpenFind,
        OpenReplace,
        FindNext,
        FindPrev,
        ReplaceNext,
        ReplaceAll,
        CloseFind,
    ]
);

/// Key context set by the editable text comparison view.
pub const DIFF_EDITOR_CONTEXT: &str = "DiffEditor";

/// Key context set by the folder comparison view.
pub const FOLDER_VIEW_CONTEXT: &str = "FolderView";

/// Registers every key binding.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-t", NewTab, None),
        KeyBinding::new("cmd-n", NewTextCompare, None),
        KeyBinding::new("alt-cmd-g", OpenGitHistory, None),
        KeyBinding::new("cmd-shift-b", Beautify, None),
        KeyBinding::new("alt-z", ToggleWordWrap, None),
        KeyBinding::new("cmd-o", OpenLeft, None),
        KeyBinding::new("cmd-shift-o", OpenRight, None),
        KeyBinding::new("cmd-shift-e", OpenFolderLeft, None),
        KeyBinding::new("cmd-shift-r", OpenFolderRight, None),
        KeyBinding::new("cmd-enter", Compare, None),
        KeyBinding::new("cmd-shift-l", CompareFolders, None),
        KeyBinding::new("cmd-shift-m", Merge, None),
        KeyBinding::new("cmd-w", CloseTab, None),
        KeyBinding::new("cmd-s", SaveFile, None),
        KeyBinding::new("alt-cmd-s", SaveAll, None),
        KeyBinding::new("cmd-]", NextTab, None),
        KeyBinding::new("cmd-[", PrevTab, None),
        KeyBinding::new("alt-down", NextDifference, None),
        KeyBinding::new("alt-up", PrevDifference, None),
        KeyBinding::new("f8", NextDifference, None),
        KeyBinding::new("f7", PrevDifference, None),
        KeyBinding::new("cmd-shift-c", CopyPaths, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("down", SelectNext, Some(FOLDER_VIEW_CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some(FOLDER_VIEW_CONTEXT)),
        KeyBinding::new("right", Expand, Some(FOLDER_VIEW_CONTEXT)),
        KeyBinding::new("left", Collapse, Some(FOLDER_VIEW_CONTEXT)),
        KeyBinding::new("enter", OpenSelected, Some(FOLDER_VIEW_CONTEXT)),
    ]);
    bind_editor_keys(cx);
}

/// Registers find-bar keys. Call after the UI component library has
/// registered its own, so these win inside the find and replace fields.
pub fn bind_find_keys(cx: &mut App) {
    use crate::text_diff_view::{FIND_QUERY_CONTEXT, REPLACE_QUERY_CONTEXT};
    let find_field = format!("{FIND_QUERY_CONTEXT} > Input");
    let replace_field = format!("{REPLACE_QUERY_CONTEXT} > Input");
    let line_break = gpui_component::input::Enter { secondary: false };
    cx.bind_keys([
        KeyBinding::new("cmd-f", OpenFind, None),
        KeyBinding::new("alt-cmd-f", OpenReplace, None),
        KeyBinding::new("cmd-g", FindNext, None),
        KeyBinding::new("cmd-shift-g", FindPrev, None),
        KeyBinding::new("enter", FindNext, Some(&find_field)),
        KeyBinding::new("shift-enter", line_break.clone(), Some(&find_field)),
        KeyBinding::new("escape", CloseFind, Some(&find_field)),
        KeyBinding::new("enter", ReplaceNext, Some(&replace_field)),
        KeyBinding::new("alt-cmd-enter", ReplaceAll, Some(&replace_field)),
        KeyBinding::new("shift-enter", line_break, Some(&replace_field)),
        KeyBinding::new("escape", CloseFind, Some(&replace_field)),
    ]);
}

fn bind_editor_keys(cx: &mut App) {
    let context = Some(DIFF_EDITOR_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, context),
        KeyBinding::new("right", MoveRight, context),
        KeyBinding::new("up", MoveUp, context),
        KeyBinding::new("down", MoveDown, context),
        KeyBinding::new("alt-left", MoveWordLeft, context),
        KeyBinding::new("alt-right", MoveWordRight, context),
        KeyBinding::new("cmd-left", MoveLineStart, context),
        KeyBinding::new("cmd-right", MoveLineEnd, context),
        KeyBinding::new("home", MoveLineStart, context),
        KeyBinding::new("end", MoveLineEnd, context),
        KeyBinding::new("cmd-up", MoveDocStart, context),
        KeyBinding::new("cmd-down", MoveDocEnd, context),
        KeyBinding::new("pageup", MovePageUp, context),
        KeyBinding::new("pagedown", MovePageDown, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("alt-shift-left", SelectWordLeft, context),
        KeyBinding::new("alt-shift-right", SelectWordRight, context),
        KeyBinding::new("cmd-shift-left", SelectLineStart, context),
        KeyBinding::new("cmd-shift-right", SelectLineEnd, context),
        KeyBinding::new("shift-home", SelectLineStart, context),
        KeyBinding::new("shift-end", SelectLineEnd, context),
        KeyBinding::new("cmd-shift-up", SelectDocStart, context),
        KeyBinding::new("cmd-shift-down", SelectDocEnd, context),
        KeyBinding::new("shift-pageup", SelectPageUp, context),
        KeyBinding::new("shift-pagedown", SelectPageDown, context),
        KeyBinding::new("cmd-a", SelectAll, context),
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("shift-backspace", Backspace, context),
        KeyBinding::new("delete", DeleteForward, context),
        KeyBinding::new("alt-backspace", DeleteWordLeft, context),
        KeyBinding::new("alt-delete", DeleteWordRight, context),
        KeyBinding::new("cmd-backspace", DeleteToLineStart, context),
        KeyBinding::new("enter", Newline, context),
        KeyBinding::new("shift-enter", Newline, context),
        KeyBinding::new("tab", Indent, context),
        KeyBinding::new("cmd-c", Copy, context),
        KeyBinding::new("cmd-x", Cut, context),
        KeyBinding::new("cmd-v", Paste, context),
        KeyBinding::new("cmd-z", Undo, context),
        KeyBinding::new("cmd-shift-z", Redo, context),
        KeyBinding::new("escape", CollapseSelection, context),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, context),
        KeyBinding::new("ctrl-alt-right", CopyBlockToRight, context),
        KeyBinding::new("ctrl-alt-left", CopyBlockToLeft, context),
        KeyBinding::new("ctrl-alt-shift-right", CopyLineToRight, context),
        KeyBinding::new("ctrl-alt-shift-left", CopyLineToLeft, context),
    ]);
}
