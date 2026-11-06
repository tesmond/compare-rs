//! Colors and metrics shared by all views (Catppuccin Mocha palette).

use gpui::{Hsla, Pixels, px, rgb, rgba};

pub const MONO_FONT: &str = "Menlo";

pub fn row_height() -> Pixels {
    px(20.0)
}

/// Code and file contents.
pub fn code_text_size() -> Pixels {
    px(13.0)
}

/// Labels on buttons, tabs, headers, and the find bar.
pub fn ui_text_size() -> Pixels {
    px(12.0)
}

/// The status bar summary.
pub fn summary_text_size() -> Pixels {
    px(11.0)
}

/// Secondary details: sizes, dates, folders, and hints.
pub fn meta_text_size() -> Pixels {
    px(10.0)
}

/// Height of toolbar and header buttons.
pub fn control_height() -> Pixels {
    px(28.0)
}

/// Height of the file headers above a comparison.
pub fn header_height() -> Pixels {
    px(32.0)
}

fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub fn base() -> Hsla {
    color(0x1e1e2e)
}

pub fn mantle() -> Hsla {
    color(0x181825)
}

pub fn crust() -> Hsla {
    color(0x11111b)
}

pub fn surface() -> Hsla {
    color(0x313244)
}

pub fn surface_bright() -> Hsla {
    color(0x45475a)
}

pub fn text() -> Hsla {
    color(0xcdd6f4)
}

pub fn subtext() -> Hsla {
    color(0xa6adc8)
}

pub fn muted() -> Hsla {
    color(0x6c7086)
}

pub fn gutter() -> Hsla {
    color(0x6c7086)
}

pub fn accent() -> Hsla {
    color(0x89b4fa)
}

pub fn accent_translucent() -> Hsla {
    rgba(0x89b4fa44).into()
}

/// Background of a toggle that is switched on.
pub fn active_bg() -> Hsla {
    rgba(0x89b4fa33).into()
}

/// Background of selected text on the side with the cursor.
pub fn selection() -> Hsla {
    rgba(0x89b4fa55).into()
}

/// Background of selected text on the other side.
pub fn selection_inactive() -> Hsla {
    rgba(0x9399b255).into()
}

/// Background of text matching the find pattern.
pub fn find_match() -> Hsla {
    rgba(0xf9e2af66).into()
}

/// Background of the selected find match.
pub fn find_current() -> Hsla {
    rgba(0xfab387aa).into()
}

pub fn cursor() -> Hsla {
    color(0xf5e0dc)
}

pub fn red() -> Hsla {
    color(0xf38ba8)
}

pub fn green() -> Hsla {
    color(0xa6e3a1)
}

pub fn yellow() -> Hsla {
    color(0xf9e2af)
}

/// Background of lines that exist only on the left (or changed lines on the left).
pub fn deleted_bg() -> Hsla {
    color(0x3c1f2b)
}

/// Background of the changed words within a deleted/changed left line.
pub fn deleted_emphasis() -> Hsla {
    color(0x803048)
}

pub fn inserted_bg() -> Hsla {
    color(0x1e3a2a)
}

pub fn inserted_emphasis() -> Hsla {
    color(0x2f7048)
}

pub fn conflict_bg() -> Hsla {
    color(0x3d3520)
}

pub fn conflict_selected_bg() -> Hsla {
    color(0x5c4e22)
}

pub fn both_changed_bg() -> Hsla {
    color(0x1f2f45)
}

/// Background of the empty filler opposite an inserted or deleted line.
pub fn filler_bg() -> Hsla {
    color(0x161622)
}
