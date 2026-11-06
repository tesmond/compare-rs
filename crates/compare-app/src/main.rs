//! compare-rs: a GPU-accelerated file and folder comparison tool.
//!
//! Usage: `compare-app [LEFT RIGHT | LEFT BASE RIGHT]`. Two paths open a file or
//! folder comparison; three paths open a three-way merge.

mod actions;
mod app_dirs;
mod button;
mod code_text;
mod crash_log;
mod document_syntax;
mod editor_line;
mod file_header;
mod folder_diff_view;
mod format;
mod hex_diff_view;
mod horizontal_scroll;
mod merge_view;
mod path_actions;
mod session_state;
mod tab;
mod text_diff_view;
mod theme;
mod workspace;

use std::path::PathBuf;

use gpui::{
    App, AppContext, Application, Bounds, Focusable, Global, Menu, MenuItem, PromptLevel,
    SharedString, TitlebarOptions, WeakEntity, WindowBounds, WindowOptions, px, size,
};

use crate::actions::{
    Beautify, CloseTab, Compare, CopyPaths, Merge, NewTab, NewTextCompare, NextDifference, NextTab,
    OpenFolderLeft, OpenFolderRight, OpenLeft, OpenRight, PrevDifference, PrevTab, Quit,
    RevertFile, SaveAll, SaveFile, ToggleIgnoreCase, ToggleIgnoreWhitespace, ToggleWordWrap,
};
use crate::actions::{
    Copy, CopyBlockToLeft, CopyBlockToRight, CopyLineToLeft, CopyLineToRight, Cut, FindNext,
    FindPrev, InsertLine, OpenFind, OpenReplace, Paste, Redo, SelectAll, Undo,
};
use crate::workspace::Workspace;

const APP_NAME: &str = "compare-rs";

fn main() {
    crash_log::install();
    let cli_paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    Application::new().with_assets(gpui_component_assets::Assets).run(|cx: &mut App| {
        actions::bind_keys(cx);
        cx.on_action(quit);
        cx.set_menus(menus());
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        actions::bind_find_keys(cx);

        let session = session_state::load().unwrap_or_default();
        let window_bounds =
            session.window.and_then(|saved| saved.to_window_bounds()).unwrap_or_else(|| {
                WindowBounds::Windowed(Bounds::centered(None, size(px(1400.0), px(900.0)), cx))
            });
        // Command-line paths replace the saved tabs.
        let restore_tabs = cli_paths.is_empty();
        let options = WindowOptions {
            window_bounds: Some(window_bounds),
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from(APP_NAME)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            let workspace = cx.new(|cx| {
                let mut workspace = Workspace::new(cx);
                workspace.watch_window(window, cx);
                if restore_tabs {
                    workspace.restore(session, cx);
                } else {
                    workspace.open_paths(cli_paths, cx);
                }
                workspace
            });
            window.focus(&workspace.read(cx).focus_handle(cx));
            cx.set_global(MainWorkspace(workspace.downgrade()));
            // Closing the window quits, so ask about unsaved edits first.
            let closing_workspace = workspace.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                closing_workspace
                    .update(cx, |workspace, cx| workspace.confirm_window_close(window, cx))
                    .unwrap_or(true)
            });
            // Save the final state synchronously on quit, since a pending
            // debounced save would be dropped.
            let saved_workspace = workspace.clone();
            cx.on_app_quit(move |cx| {
                if let Err(error) = session_state::save(&saved_workspace.read(cx).session()) {
                    crash_log::record_error("Saving session", &error.to_string());
                }
                async {}
            })
            .detach();
            if let Some(log) = crash_log::take_unseen_crash() {
                let answer = window.prompt(
                    PromptLevel::Warning,
                    &format!("{APP_NAME} quit unexpectedly last time."),
                    Some("A crash report was saved on this Mac. Nothing was sent anywhere."),
                    &["Show Crash Log", "Dismiss"],
                    cx,
                );
                cx.spawn(async move |cx| {
                    if answer.await == Ok(0) {
                        cx.update(|cx| cx.reveal_path(&log)).ok();
                    }
                })
                .detach();
            }
            cx.new(|cx| gpui_component::Root::new(workspace, window, cx))
        });
        if let Err(error) = opened {
            eprintln!("failed to open the main window: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}

/// The workspace of the main window, so app-level actions can reach it.
struct MainWorkspace(WeakEntity<Workspace>);

impl Global for MainWorkspace {}

/// Quits, letting the workspace ask about unsaved edits first.
fn quit(_: &Quit, cx: &mut App) {
    let Some(workspace) = cx.try_global::<MainWorkspace>().and_then(|main| main.0.upgrade()) else {
        cx.quit();
        return;
    };
    // Run once the current event is done, when the window is free to update.
    cx.defer(move |cx| {
        let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
            cx.quit();
            return;
        };
        let asked = window.update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| workspace.request_quit(window, cx));
        });
        if asked.is_err() {
            cx.quit();
        }
    });
}

fn menus() -> Vec<Menu> {
    vec![
        Menu { name: APP_NAME.into(), items: vec![MenuItem::action("Quit", Quit)] },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Tab", NewTab),
                MenuItem::action("New Text Compare", NewTextCompare),
                MenuItem::separator(),
                MenuItem::action("Open Left File…", OpenLeft),
                MenuItem::action("Open Right File…", OpenRight),
                MenuItem::action("Open Left Folder…", OpenFolderLeft),
                MenuItem::action("Open Right Folder…", OpenFolderRight),
                MenuItem::separator(),
                MenuItem::action("Three-way Merge…", Merge),
                MenuItem::separator(),
                MenuItem::action("Save", SaveFile),
                MenuItem::action("Save Both Sides", SaveAll),
                MenuItem::action("Revert", RevertFile),
                MenuItem::separator(),
                MenuItem::action("Close Tab", CloseTab),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo", Undo),
                MenuItem::action("Redo", Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", Cut),
                MenuItem::action("Copy", Copy),
                MenuItem::action("Paste", Paste),
                MenuItem::separator(),
                MenuItem::action("Select All", SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find…", OpenFind),
                MenuItem::action("Find and Replace…", OpenReplace),
                MenuItem::action("Find Next", FindNext),
                MenuItem::action("Find Previous", FindPrev),
                MenuItem::separator(),
                MenuItem::action("Insert Line", InsertLine),
                MenuItem::action("Copy Line Left", CopyLineToLeft),
                MenuItem::action("Copy Line Right", CopyLineToRight),
                MenuItem::action("Copy Section Left", CopyBlockToLeft),
                MenuItem::action("Copy Section Right", CopyBlockToRight),
            ],
        },
        Menu {
            name: "Compare".into(),
            items: vec![
                MenuItem::action("Compare", Compare),
                MenuItem::action("Next Difference", NextDifference),
                MenuItem::action("Previous Difference", PrevDifference),
                MenuItem::separator(),
                MenuItem::action("Ignore Whitespace", ToggleIgnoreWhitespace),
                MenuItem::action("Ignore Case", ToggleIgnoreCase),
                MenuItem::action("Beautify JSON / Dicts / \\n", Beautify),
                MenuItem::action("Word Wrap", ToggleWordWrap),
                MenuItem::separator(),
                MenuItem::action("Copy Paths", CopyPaths),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PrevTab),
            ],
        },
    ]
}
