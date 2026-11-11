//! Rendering a line of code with syntax colors and highlighted ranges.

use std::ops::Range;

use gpui::{FontStyle, HighlightStyle, Hsla, SharedString, StyledText, UnderlineStyle, px, rgb};

use compare_syntax::{Token, TokenKind};

/// Color for each kind of token (Catppuccin Mocha).
pub fn token_color(kind: TokenKind) -> Hsla {
    let hex = match kind {
        TokenKind::Comment => 0x7f849c,
        TokenKind::String => 0xa6e3a1,
        TokenKind::Number | TokenKind::Constant => 0xfab387,
        TokenKind::Keyword => 0xcba6f7,
        TokenKind::Type => 0xf9e2af,
        TokenKind::Function => 0x89b4fa,
        TokenKind::Key => 0x89dceb,
        TokenKind::Meta => 0xf5c2e7,
    };
    rgb(hex).into()
}

/// A range of a line drawn with a background and/or underline. Where marks
/// overlap, later ones win.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    /// Byte range in the displayed text.
    pub range: Range<usize>,
    pub background: Option<Hsla>,
    pub underline: Option<Hsla>,
    /// Text color, overriding the syntax color.
    pub foreground: Option<Hsla>,
}

impl Mark {
    pub fn background(range: Range<usize>, color: Hsla) -> Self {
        Self { range, background: Some(color), underline: None, foreground: None }
    }

    pub fn underline(range: Range<usize>, color: Hsla) -> Self {
        Self { range, background: None, underline: Some(color), foreground: None }
    }
}

/// Builds styled text from syntax `tokens` and `marks`. Ranges are byte
/// offsets into `text`.
pub fn code_text(text: String, tokens: &[Token], marks: &[Mark]) -> StyledText {
    let highlights = merge_highlights(text.len(), tokens, marks);
    StyledText::new(SharedString::from(text)).with_highlights(highlights)
}

/// Combines token colors and marks into the sorted, non-overlapping list
/// that `StyledText` requires.
fn merge_highlights(
    len: usize,
    tokens: &[Token],
    marks: &[Mark],
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut bounds: Vec<usize> = Vec::with_capacity((tokens.len() + marks.len()) * 2);
    for range in tokens.iter().map(|t| &t.range).chain(marks.iter().map(|m| &m.range)) {
        if range.start < range.end && range.end <= len {
            bounds.push(range.start);
            bounds.push(range.end);
        }
    }
    bounds.sort_unstable();
    bounds.dedup();

    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut token_index = 0;
    for pair in bounds.windows(2) {
        let segment = pair[0]..pair[1];
        while tokens.get(token_index).is_some_and(|t| t.range.end <= segment.start) {
            token_index += 1;
        }
        let token = tokens.get(token_index).filter(|t| t.range.start <= segment.start);
        let mut background = None;
        let mut underline = None;
        let mut foreground = None;
        for mark in marks {
            if mark.range.start <= segment.start && segment.end <= mark.range.end {
                background = mark.background.or(background);
                underline = mark.underline.or(underline);
                foreground = mark.foreground.or(foreground);
            }
        }
        if token.is_none() && background.is_none() && underline.is_none() && foreground.is_none() {
            continue;
        }
        let style = HighlightStyle {
            color: foreground.or(token.map(|t| token_color(t.kind))),
            font_style: token.filter(|t| t.kind == TokenKind::Comment).map(|_| FontStyle::Italic),
            background_color: background,
            underline: underline.map(|color| UnderlineStyle {
                color: Some(color),
                thickness: px(1.0),
                wavy: false,
            }),
            ..Default::default()
        };
        highlights.push((segment, style));
    }
    highlights
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_ranges_are_split() {
        let tokens = [Token { range: 0..5, kind: TokenKind::Keyword }];
        let marks = [Mark::background(3..8, rgb(0x000000).into())];
        let merged = merge_highlights(10, &tokens, &marks);
        let ranges: Vec<Range<usize>> = merged.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, [0..3, 3..5, 5..8]);
        assert!(merged[0].1.background_color.is_none());
        assert!(merged[1].1.background_color.is_some() && merged[1].1.color.is_some());
        assert!(merged[2].1.color.is_none());
    }

    #[test]
    fn later_marks_win() {
        let red: Hsla = rgb(0xff0000).into();
        let blue: Hsla = rgb(0x0000ff).into();
        let marks = [Mark::background(0..4, red), Mark::background(2..3, blue)];
        let merged = merge_highlights(4, &[], &marks);
        let colors: Vec<Option<Hsla>> = merged.iter().map(|(_, s)| s.background_color).collect();
        assert_eq!(colors, [Some(red), Some(blue), Some(red)]);
    }
}
