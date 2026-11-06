//! Editable text held as lines, remembering how it was stored on disk.

use std::ops::Range;

use crate::text_document::{Encoding, TextDocument};

/// The terminator after a line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

/// A place in the text: a line index and a byte offset within that line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl Position {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }

    /// The position just after `text` when it is inserted at `self`.
    pub fn after_insert(self, text: &str) -> Self {
        match text.rfind('\n') {
            None => Self::new(self.line, self.column + text.len()),
            Some(last) => {
                let newlines = memchr::memchr_iter(b'\n', text.as_bytes()).count();
                Self::new(self.line + newlines, text.len() - last - 1)
            }
        }
    }
}

/// Which lines a [`EditBuffer::replace`] call changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineChange {
    /// First line touched.
    pub first_line: usize,
    /// Number of lines, starting at `first_line`, that were replaced.
    pub old_count: usize,
    /// Number of lines that replaced them.
    pub new_count: usize,
}

/// Text split into lines, with each line's terminator and the file encoding
/// kept so the text can be written back unchanged apart from edits.
///
/// There is always at least one line. Text ending in a newline has a final
/// empty line, as in most editors, so every position can hold the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditBuffer {
    lines: Vec<String>,
    /// `endings[i]` terminates `lines[i]`; the last line has no terminator.
    endings: Vec<LineEnding>,
    /// Used for newly typed line breaks: the file's most common terminator.
    default_ending: LineEnding,
    encoding: Encoding,
}

impl Default for EditBuffer {
    fn default() -> Self {
        Self::from_text("", Encoding::Utf8)
    }
}

impl EditBuffer {
    /// Splits `text` into lines, recording each `\n` or `\r\n` terminator.
    pub fn from_text(text: &str, encoding: Encoding) -> Self {
        let mut lines = Vec::with_capacity(text.len() / 40 + 1);
        let mut endings = Vec::with_capacity(text.len() / 40 + 1);
        let mut start = 0;
        for newline in memchr::memchr_iter(b'\n', text.as_bytes()) {
            let crlf = newline > start && text.as_bytes()[newline - 1] == b'\r';
            let end = if crlf { newline - 1 } else { newline };
            lines.push(text[start..end].to_owned());
            endings.push(if crlf { LineEnding::CrLf } else { LineEnding::Lf });
            start = newline + 1;
        }
        lines.push(text[start..].to_owned());
        let crlf_count = endings.iter().filter(|&&e| e == LineEnding::CrLf).count();
        let default_ending =
            if crlf_count * 2 > endings.len() { LineEnding::CrLf } else { LineEnding::Lf };
        Self { lines, endings, default_ending, encoding }
    }

    /// Builds a buffer from a decoded document, keeping its encoding.
    pub fn from_document(document: &TextDocument) -> Self {
        Self::from_text(document.text(), document.encoding())
    }

    /// Number of lines (at least one).
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Always false: a buffer has at least one (possibly empty) line.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Returns line `index` without its terminator.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    pub fn line(&self, index: usize) -> &str {
        &self.lines[index]
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The end of the text.
    pub fn end(&self) -> Position {
        let last = self.lines.len() - 1;
        Position::new(last, self.lines[last].len())
    }

    /// Moves `position` to the nearest valid place: within the text and on a
    /// character boundary.
    pub fn clamp(&self, position: Position) -> Position {
        if position.line >= self.lines.len() {
            return self.end();
        }
        let line = &self.lines[position.line];
        let mut column = position.column.min(line.len());
        while !line.is_char_boundary(column) {
            column -= 1;
        }
        Position::new(position.line, column)
    }

    /// The text between two positions, with lines joined by `\n`.
    pub fn text_in(&self, range: Range<Position>) -> String {
        let (start, end) = (self.clamp(range.start), self.clamp(range.end));
        if start.line == end.line {
            return self.lines[start.line][start.column..end.column.max(start.column)].to_owned();
        }
        let mut text = self.lines[start.line][start.column..].to_owned();
        for line in &self.lines[start.line + 1..end.line] {
            text.push('\n');
            text.push_str(line);
        }
        text.push('\n');
        text.push_str(&self.lines[end.line][..end.column]);
        text
    }

    /// Replaces the text in `range` with `text` (lines separated by `\n`).
    /// New line breaks use the file's most common terminator.
    pub fn replace(&mut self, range: Range<Position>, text: &str) -> LineChange {
        let (start, end) = (self.clamp(range.start), self.clamp(range.end));
        let (start, end) = if end < start { (end, start) } else { (start, end) };
        let suffix = self.lines[end.line][end.column..].to_owned();
        let mut new_lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
        let first = std::mem::take(&mut new_lines[0]);
        new_lines[0] = format!("{}{first}", &self.lines[start.line][..start.column]);
        new_lines.last_mut().expect("split yields a line").push_str(&suffix);

        let new_count = new_lines.len();
        self.lines.splice(start.line..=end.line, new_lines);
        self.endings
            .splice(start.line..end.line, std::iter::repeat_n(self.default_ending, new_count - 1));
        LineChange { first_line: start.line, old_count: end.line - start.line + 1, new_count }
    }

    /// The text with lines joined by `\n`, ignoring the original terminators.
    pub fn joined(&self) -> String {
        self.lines.join("\n")
    }

    /// The text with each line's original terminator.
    pub fn to_text(&self) -> String {
        let mut text = String::with_capacity(self.lines.iter().map(|l| l.len() + 2).sum());
        for (line, ending) in self.lines.iter().zip(&self.endings) {
            text.push_str(line);
            text.push_str(ending.as_str());
        }
        text.push_str(self.lines.last().expect("at least one line"));
        text
    }

    /// The bytes to write to disk: the text in its original encoding.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.encoding.encode(&self.to_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(text: &str) -> EditBuffer {
        EditBuffer::from_text(text, Encoding::Utf8)
    }

    fn pos(line: usize, column: usize) -> Position {
        Position::new(line, column)
    }

    #[test]
    fn round_trips_mixed_line_endings() {
        let text = "a\r\nb\nc\r\n";
        let buffer = buffer(text);
        assert_eq!(buffer.lines(), ["a", "b", "c", ""]);
        assert_eq!(buffer.to_text(), text);
        assert_eq!(EditBuffer::default().lines(), [""]);
    }

    #[test]
    fn replaces_within_a_line() {
        let mut buffer = buffer("hello world\n");
        let change = buffer.replace(pos(0, 6)..pos(0, 11), "there");
        assert_eq!(buffer.to_text(), "hello there\n");
        assert_eq!(change, LineChange { first_line: 0, old_count: 1, new_count: 1 });
    }

    #[test]
    fn inserted_breaks_use_the_common_terminator() {
        let mut buffer = buffer("one\r\ntwo\r\n");
        let change = buffer.replace(pos(0, 3)..pos(0, 3), "\nnew");
        assert_eq!(buffer.to_text(), "one\r\nnew\r\ntwo\r\n");
        assert_eq!(change, LineChange { first_line: 0, old_count: 1, new_count: 2 });
    }

    #[test]
    fn deletes_across_lines_keeping_the_last_terminator() {
        let mut buffer = buffer("a\nb\r\nc\r\nd");
        assert_eq!(buffer.text_in(pos(0, 1)..pos(2, 0)), "\nb\n");
        let change = buffer.replace(pos(0, 1)..pos(2, 0), "");
        assert_eq!(buffer.to_text(), "ac\r\nd");
        assert_eq!(change, LineChange { first_line: 0, old_count: 3, new_count: 1 });
    }

    #[test]
    fn positions_after_insert() {
        assert_eq!(pos(2, 3).after_insert("ab"), pos(2, 5));
        assert_eq!(pos(2, 3).after_insert("ab\ncd\nxyz"), pos(4, 3));
    }

    #[test]
    fn clamps_to_char_boundaries() {
        let buffer = buffer("héllo");
        assert_eq!(buffer.clamp(pos(0, 2)), pos(0, 1));
        assert_eq!(buffer.clamp(pos(9, 0)), pos(0, 6));
    }

    #[test]
    fn saves_in_the_original_encoding() {
        let document = TextDocument::decode(b"\xff\xfeh\x00i\x00");
        let mut buffer = EditBuffer::from_document(&document);
        buffer.replace(pos(0, 2)..pos(0, 2), "!");
        assert_eq!(buffer.to_bytes(), b"\xff\xfeh\x00i\x00!\x00");
    }
}
