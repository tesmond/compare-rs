//! Cursor movement within an [`EditBuffer`], independent of layout.

use unicode_segmentation::UnicodeSegmentation;

use compare_core::{EditBuffer, Position};

/// The position one character (grapheme) before `position`, moving to the
/// end of the previous line from the start of a line.
pub fn prev_char(buffer: &EditBuffer, position: Position) -> Position {
    if position.column == 0 {
        return match position.line {
            0 => position,
            line => Position::new(line - 1, buffer.line(line - 1).len()),
        };
    }
    let line = &buffer.line(position.line)[..position.column];
    let column = line.grapheme_indices(true).next_back().map_or(0, |(index, _)| index);
    Position::new(position.line, column)
}

/// The position one character (grapheme) after `position`, moving to the
/// start of the next line from the end of a line.
pub fn next_char(buffer: &EditBuffer, position: Position) -> Position {
    let line = buffer.line(position.line);
    if position.column >= line.len() {
        return if position.line + 1 < buffer.len() {
            Position::new(position.line + 1, 0)
        } else {
            position
        };
    }
    let rest = &line[position.column..];
    let step = rest.graphemes(true).next().map_or(rest.len(), str::len);
    Position::new(position.line, position.column + step)
}

/// What kind of run a character belongs to when moving by words.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Space,
    Word,
    Punctuation,
}

fn class(ch: char) -> CharClass {
    if ch.is_whitespace() {
        CharClass::Space
    } else if ch.is_alphanumeric() || ch == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

/// The start of the word before `position` (skipping spaces first), or the
/// end of the previous line from the start of a line.
pub fn prev_word(buffer: &EditBuffer, position: Position) -> Position {
    if position.column == 0 {
        return prev_char(buffer, position);
    }
    let line = &buffer.line(position.line)[..position.column];
    let mut chars = line.char_indices().rev().peekable();
    while chars.next_if(|&(_, ch)| class(ch) == CharClass::Space).is_some() {}
    let Some(&(_, first)) = chars.peek() else {
        return Position::new(position.line, 0);
    };
    let run = class(first);
    let mut column = 0;
    for (index, ch) in chars {
        if class(ch) != run {
            column = index + ch.len_utf8();
            break;
        }
    }
    Position::new(position.line, column)
}

/// The end of the word after `position` (skipping spaces first), or the
/// start of the next line from the end of a line.
pub fn next_word(buffer: &EditBuffer, position: Position) -> Position {
    let line = buffer.line(position.line);
    if position.column >= line.len() {
        return next_char(buffer, position);
    }
    let mut chars = line[position.column..].char_indices().peekable();
    while chars.next_if(|&(_, ch)| class(ch) == CharClass::Space).is_some() {}
    let Some(&(_, first)) = chars.peek() else {
        return Position::new(position.line, line.len());
    };
    let run = class(first);
    let end = chars.find(|&(_, ch)| class(ch) != run).map_or(line.len() - position.column, |c| c.0);
    Position::new(position.line, position.column + end)
}

/// The word (or run of spaces or punctuation) around `position`, for
/// double-click selection.
pub fn word_at(buffer: &EditBuffer, position: Position) -> (Position, Position) {
    let line = buffer.line(position.line);
    let Some(ch) = line[position.column..]
        .chars()
        .next()
        .or_else(|| line[..position.column].chars().next_back())
    else {
        return (position, position);
    };
    let run = class(ch);
    let start = line[..position.column]
        .char_indices()
        .rev()
        .find(|&(_, c)| class(c) != run)
        .map_or(0, |(index, c)| index + c.len_utf8());
    let end = line[position.column..]
        .char_indices()
        .find(|&(_, c)| class(c) != run)
        .map_or(line.len(), |(index, _)| position.column + index);
    (Position::new(position.line, start), Position::new(position.line, end))
}

/// "Smart home": the first non-blank character of the line, or the very
/// start if the cursor is already there.
pub fn line_start(buffer: &EditBuffer, position: Position) -> Position {
    let line = buffer.line(position.line);
    let indent = line.len() - line.trim_start().len();
    let column = if position.column == indent { 0 } else { indent };
    Position::new(position.line, column)
}

/// The leading whitespace of `line`, to carry onto a new line.
pub fn indentation(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

#[cfg(test)]
mod tests {
    use compare_core::Encoding;

    use super::*;

    fn buffer(text: &str) -> EditBuffer {
        EditBuffer::from_text(text, Encoding::Utf8)
    }

    fn pos(line: usize, column: usize) -> Position {
        Position::new(line, column)
    }

    #[test]
    fn chars_cross_lines_and_graphemes() {
        let buffer = buffer("ae\u{301}\nb");
        assert_eq!(next_char(&buffer, pos(0, 1)), pos(0, 4));
        assert_eq!(prev_char(&buffer, pos(0, 4)), pos(0, 1));
        assert_eq!(next_char(&buffer, pos(0, 4)), pos(1, 0));
        assert_eq!(prev_char(&buffer, pos(1, 0)), pos(0, 4));
        assert_eq!(prev_char(&buffer, pos(0, 0)), pos(0, 0));
        assert_eq!(next_char(&buffer, pos(1, 1)), pos(1, 1));
    }

    #[test]
    fn words_skip_spaces_then_one_run() {
        let buffer = buffer("let foo_bar = x.y;");
        assert_eq!(next_word(&buffer, pos(0, 3)), pos(0, 11));
        assert_eq!(next_word(&buffer, pos(0, 11)), pos(0, 13));
        assert_eq!(prev_word(&buffer, pos(0, 11)), pos(0, 4));
        assert_eq!(prev_word(&buffer, pos(0, 4)), pos(0, 0));
        assert_eq!(prev_word(&buffer, pos(0, 18)), pos(0, 17));
    }

    #[test]
    fn double_click_word() {
        let buffer = buffer("call(foo_bar)");
        assert_eq!(word_at(&buffer, pos(0, 7)), (pos(0, 5), pos(0, 12)));
        assert_eq!(word_at(&buffer, pos(0, 13)), (pos(0, 12), pos(0, 13)));
    }

    #[test]
    fn smart_home_toggles() {
        let buffer = buffer("    code");
        assert_eq!(line_start(&buffer, pos(0, 6)), pos(0, 4));
        assert_eq!(line_start(&buffer, pos(0, 4)), pos(0, 0));
        assert_eq!(indentation("\t  x"), "\t  ");
    }
}
