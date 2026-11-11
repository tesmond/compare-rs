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

/// Background of text matching the find pattern. Solid and dark, so the
/// light match text stays readable over any line background (a
/// translucent yellow blended to the same brightness as grey text).
pub fn find_match() -> Hsla {
    color(0x5a4a1f)
}

/// Text of a find match that is not the selected one.
pub fn find_match_text() -> Hsla {
    text()
}

/// Background of the selected find match: solid, so it stands out from
/// the other matches and from the selection.
pub fn find_current() -> Hsla {
    color(0xfab387)
}

/// Text of the selected find match, dark to read on its background.
pub fn find_current_text() -> Hsla {
    color(0x11111b)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance.
    fn luminance(color: Hsla) -> f32 {
        let rgb = color.to_rgb();
        let linear = |c: f32| {
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
    }

    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn find_highlights_keep_text_readable() {
        // WCAG AA for normal text.
        const MINIMUM: f32 = 4.5;
        let current = contrast(find_current_text(), find_current());
        let other = contrast(find_match_text(), find_match());
        assert!(current >= MINIMUM, "current match contrast {current:.2}");
        assert!(other >= MINIMUM, "other match contrast {other:.2}");
        // The two kinds of match must look clearly different.
        assert!(contrast(find_current(), find_match()) >= 3.0);
    }
}
