//! Context-menu operations on file paths.

use std::path::{Path, PathBuf};

use gpui::{
    App, ClipboardItem, Div, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Styled, div,
};
use gpui_component::menu::{ContextMenuExt, PopupMenu};

use crate::actions::{CopyLeftPath, CopyRightPath, RevealLeft, RevealRight};

/// Copies `path` to the clipboard.
pub fn copy_path(path: &Path, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
}

/// Shows `path` in Finder.
pub fn reveal(path: &Path, cx: &mut App) {
    cx.reveal_path(path);
}

/// Adds the standard path items for a left/right comparison to `menu`.
/// `has_left` and `has_right` disable items for a side that is absent.
pub fn add_path_items(menu: PopupMenu, has_left: bool, has_right: bool) -> PopupMenu {
    menu.menu_with_disabled("Copy Left Path", Box::new(CopyLeftPath), !has_left)
        .menu_with_disabled("Copy Right Path", Box::new(CopyRightPath), !has_right)
        .separator()
        .menu_with_disabled("Reveal Left in Finder", Box::new(RevealLeft), !has_left)
        .menu_with_disabled("Reveal Right in Finder", Box::new(RevealRight), !has_right)
}

/// Wraps `content` so a right-click shows the path menu for a fixed pair of
/// files, and the menu's actions copy or reveal those files.
pub fn with_file_pair_menu(
    content: Div,
    focus: &FocusHandle,
    left: Option<PathBuf>,
    right: Option<PathBuf>,
) -> impl IntoElement {
    let (has_left, has_right) = (left.is_some(), right.is_some());
    let menu_focus = focus.clone();
    let click_focus = focus.clone();
    let path_action = |path: Option<PathBuf>, act: fn(&Path, &mut App)| {
        move |cx: &mut App| {
            if let Some(path) = &path {
                act(path, cx);
            }
        }
    };
    let copy_left = path_action(left.clone(), copy_path);
    let copy_right = path_action(right.clone(), copy_path);
    let reveal_left = path_action(left, reveal);
    let reveal_right = path_action(right, reveal);
    div()
        .track_focus(focus)
        .size_full()
        .on_mouse_down(MouseButton::Right, move |_, window, _| window.focus(&click_focus))
        .on_action(move |_: &CopyLeftPath, _, cx| copy_left(cx))
        .on_action(move |_: &CopyRightPath, _, cx| copy_right(cx))
        .on_action(move |_: &RevealLeft, _, cx| reveal_left(cx))
        .on_action(move |_: &RevealRight, _, cx| reveal_right(cx))
        .child(content)
        .context_menu(move |menu, _, _| {
            add_path_items(menu.action_context(menu_focus.clone()), has_left, has_right)
        })
}
