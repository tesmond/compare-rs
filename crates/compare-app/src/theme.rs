//! Colors and metrics shared by all views (Catppuccin Mocha palette).

use gpui::{Hsla, Pixels, px, rgb, rgba};

pub const MONO_FONT: &str = "Menlo";

pub fn row_height() -> Pixels {
    px(18.0)
}

pub fn code_text_size() -> Pixels {
    px(12.0)
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
    color(0x585b70)
}

pub fn accent() -> Hsla {
    color(0x89b4fa)
}

pub fn accent_translucent() -> Hsla {
    rgba(0x89b4fa44).into()
}

/// Background of selected text on the side with the cursor.
pub fn selection() -> Hsla {
    rgba(0x89b4fa55).into()
}

/// Background of selected text on the other side.
pub fn selection_inactive() -> Hsla {
    rgba(0x9399b233).into()
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
