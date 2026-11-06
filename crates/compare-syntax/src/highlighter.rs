//! The line-oriented lexer.

use std::ops::Range;

use crate::language::{Language, LanguageSpec, StringDelimiter};
use crate::token::{LineState, Token, TokenKind};

/// Highlights lines of one language.
#[derive(Clone, Copy, Debug)]
pub struct Highlighter {
    spec: &'static LanguageSpec,
}

impl Highlighter {
    pub fn new(language: Language) -> Self {
        Self { spec: language.spec }
    }

    /// Returns the lexer state at the start of each line.
    ///
    /// This is a fast pass that only tracks comments and strings, so it can
    /// run over whole files.
    pub fn line_states<'a>(&self, lines: impl IntoIterator<Item = &'a str>) -> Vec<LineState> {
        let mut state = LineState::Normal;
        let mut states = Vec::new();
        for line in lines {
            states.push(state);
            state = Lexer { spec: self.spec, line, tokens: None }.run(state);
        }
        states
    }

    /// Returns the lexer state at the end of `line`, given the state at its
    /// start.
    pub fn end_state(&self, line: &str, state: LineState) -> LineState {
        Lexer { spec: self.spec, line, tokens: None }.run(state)
    }

    /// Returns the tokens of `line`, given the state at its start.
    ///
    /// Tokens are sorted and never overlap. Text not covered by a token is
    /// plain.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    /// use compare_syntax::{Highlighter, Language, LineState, TokenKind};
    ///
    /// let rust = Language::from_path(Path::new("main.rs")).expect("Rust is supported");
    /// let tokens = Highlighter::new(rust).highlight_line("let x = 1; // one", LineState::Normal);
    /// let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind).collect();
    /// assert_eq!(kinds, [TokenKind::Keyword, TokenKind::Number, TokenKind::Comment]);
    /// ```
    pub fn highlight_line(&self, line: &str, state: LineState) -> Vec<Token> {
        let mut tokens = Vec::new();
        Lexer { spec: self.spec, line, tokens: Some(&mut tokens) }.run(state);
        tokens
    }
}

struct Lexer<'a> {
    spec: &'static LanguageSpec,
    line: &'a str,
    /// `None` when only the end-of-line state is needed.
    tokens: Option<&'a mut Vec<Token>>,
}

impl Lexer<'_> {
    fn emit(&mut self, range: Range<usize>, kind: TokenKind) {
        if let Some(tokens) = self.tokens.as_deref_mut()
            && !range.is_empty()
        {
            tokens.push(Token { range, kind });
        }
    }

    /// Lexes the line and returns the state at its end.
    fn run(mut self, state: LineState) -> LineState {
        let bytes = self.line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        // Finish a construct carried over from the previous line.
        match state {
            LineState::Normal => {}
            LineState::BlockComment => {
                let Some((_, close)) = self.spec.block_comment else {
                    return LineState::Normal;
                };
                match find(bytes, 0, close) {
                    Some(end) => {
                        self.emit(0..end + close.len(), TokenKind::Comment);
                        i = end + close.len();
                    }
                    None => {
                        self.emit(0..len, TokenKind::Comment);
                        return LineState::BlockComment;
                    }
                }
            }
            LineState::String(index) => {
                let Some(delimiter) = self.spec.strings.get(usize::from(index)) else {
                    return LineState::Normal;
                };
                match string_end(bytes, 0, delimiter) {
                    Some(end) => {
                        self.emit(0..end, TokenKind::String);
                        i = end;
                    }
                    None => {
                        self.emit(0..len, TokenKind::String);
                        return state;
                    }
                }
            }
        }

        if state == LineState::Normal {
            let indent = bytes.iter().take_while(|b| b.is_ascii_whitespace()).count();
            if let Some(prefix) = self.spec.meta_line_prefix
                && bytes[indent..].starts_with(prefix.as_bytes())
            {
                self.emit(indent..len, TokenKind::Meta);
                return LineState::Normal;
            }
            if self.spec.keys {
                i = self.leading_key(indent);
            }
        }

        while i < len {
            let byte = bytes[i];
            if byte.is_ascii_whitespace() {
                i += 1;
                continue;
            }
            let rest = &bytes[i..];

            if self.is_line_comment(i) {
                self.emit(i..len, TokenKind::Comment);
                return LineState::Normal;
            }

            if let Some((open, close)) = self.spec.block_comment
                && rest.starts_with(open.as_bytes())
            {
                match find(bytes, i + open.len(), close) {
                    Some(end) => {
                        self.emit(i..end + close.len(), TokenKind::Comment);
                        i = end + close.len();
                        continue;
                    }
                    None => {
                        self.emit(i..len, TokenKind::Comment);
                        return LineState::BlockComment;
                    }
                }
            }

            let string = self
                .spec
                .strings
                .iter()
                .enumerate()
                .find(|(_, delimiter)| rest.starts_with(delimiter.open.as_bytes()));
            if let Some((index, delimiter)) = string {
                let start = i;
                match string_end(bytes, i + delimiter.open.len(), delimiter) {
                    Some(end) if !delimiter.char_literal || is_char_literal(&bytes[start..end]) => {
                        let kind = if self.spec.keys && next_non_space(bytes, end) == Some(b':') {
                            TokenKind::Key
                        } else {
                            TokenKind::String
                        };
                        self.emit(start..end, kind);
                        i = end;
                        continue;
                    }
                    Some(_) => {}
                    None if delimiter.char_literal => {}
                    None => {
                        self.emit(start..len, TokenKind::String);
                        return if delimiter.multiline {
                            LineState::String(index as u8)
                        } else {
                            LineState::Normal
                        };
                    }
                }
                // Not a string after all (e.g. a Rust lifetime): skip the quote.
                i += delimiter.open.len();
                continue;
            }

            if byte.is_ascii_digit() {
                let start = i;
                i += 1;
                while i < len
                    && (is_ident_byte(bytes[i])
                        || (bytes[i] == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)))
                {
                    i += 1;
                }
                self.emit(start..i, TokenKind::Number);
                continue;
            }

            if self.spec.meta_prefix == Some(byte as char) {
                i = self.meta_token(i);
                continue;
            }

            if is_ident_byte(byte) {
                let start = i;
                while i < len && (is_ident_byte(bytes[i]) || bytes[i] == b'?' && i == start + 7) {
                    i += 1;
                }
                if self.tokens.is_some()
                    && let Some(kind) = self.classify(start..i)
                {
                    self.emit(start..i, kind);
                }
                continue;
            }

            i += utf8_len(byte);
        }
        LineState::Normal
    }

    /// Returns true if a line comment starts at byte `i`.
    fn is_line_comment(&self, i: usize) -> bool {
        let bytes = self.line.as_bytes();
        self.spec.line_comments.iter().any(|marker| {
            bytes[i..].starts_with(marker.as_bytes())
                // `#` only starts a comment at a word boundary (`a#b` is not one).
                && (*marker != "#" || i == 0 || bytes[i - 1].is_ascii_whitespace())
        })
    }

    /// Highlights a `key:` or `key =` at the start of the line and returns the
    /// byte offset to continue lexing from.
    fn leading_key(&mut self, indent: usize) -> usize {
        let bytes = self.line.as_bytes();
        let mut start = indent;
        if bytes[start..].starts_with(b"- ") {
            start += 2;
            while start < bytes.len() && bytes[start] == b' ' {
                start += 1;
            }
        }
        let mut end = start;
        while end < bytes.len()
            && (is_ident_byte(bytes[end]) || matches!(bytes[end], b'-' | b'.' | b'/'))
        {
            end += 1;
        }
        match next_non_space(bytes, end) {
            Some(b':' | b'=') if end > start => {
                self.emit(start..end, TokenKind::Key);
                end
            }
            _ => indent,
        }
    }

    /// Consumes a meta token starting at `start` (`#[attr]`, `@decorator`,
    /// `$VAR`, `${VAR}`, `&anchor`) and returns the offset after it.
    fn meta_token(&mut self, start: usize) -> usize {
        let bytes = self.line.as_bytes();
        let mut end = start + 1;
        if bytes.get(end) == Some(&b'!') {
            end += 1;
        }
        match bytes.get(end) {
            Some(b'[') => end = find(bytes, end, "]").map_or(bytes.len(), |close| close + 1),
            Some(b'{') => end = find(bytes, end, "}").map_or(bytes.len(), |close| close + 1),
            _ => {
                while end < bytes.len() && (is_ident_byte(bytes[end]) || bytes[end] == b'.') {
                    end += 1;
                }
            }
        }
        if end == start + 1 {
            // A lone prefix character (e.g. `&` used as an operator).
            return end;
        }
        self.emit(start..end, TokenKind::Meta);
        end
    }

    fn classify(&self, range: Range<usize>) -> Option<TokenKind> {
        let word = &self.line[range.clone()];
        let bytes = self.line.as_bytes();
        let is_keyword = if self.spec.case_insensitive_keywords {
            self.spec.keywords.iter().any(|k| k.eq_ignore_ascii_case(word))
        } else {
            self.spec.keywords.contains(&word)
        };
        if is_keyword {
            return Some(TokenKind::Keyword);
        }
        if self.spec.constants.contains(&word) {
            return Some(TokenKind::Constant);
        }
        if self.spec.types.contains(&word) {
            return Some(TokenKind::Type);
        }
        if self.spec.tags
            && previous_non_space(bytes, range.start).is_some_and(|b| b == b'<' || b == b'/')
        {
            return Some(TokenKind::Key);
        }
        if next_non_space(bytes, range.end) == Some(b'(') {
            return Some(TokenKind::Function);
        }
        let capitalized = word.as_bytes()[0].is_ascii_uppercase() && word.len() > 1;
        if self.spec.capitalized_types && capitalized {
            return Some(TokenKind::Type);
        }
        None
    }
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// Returns true if `literal` (quotes included) holds one character or one
/// escape sequence.
fn is_char_literal(literal: &[u8]) -> bool {
    let body = &literal[1..literal.len() - 1];
    match body.first() {
        Some(b'\\') => body.len() <= 10,
        Some(&first) => body.len() == utf8_len(first),
        None => false,
    }
}

fn utf8_len(first_byte: u8) -> usize {
    match first_byte {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

/// Finds `needle` in `bytes` at or after `from`.
fn find(bytes: &[u8], from: usize, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    (from..bytes.len()).find(|&i| bytes[i..].starts_with(needle))
}

/// Returns the offset just past the closing delimiter of a string whose body
/// starts at `from`, honoring escapes.
fn string_end(bytes: &[u8], from: usize, delimiter: &StringDelimiter) -> Option<usize> {
    let close = delimiter.close.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if delimiter.escapes && bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(close) {
            return Some(i + close.len());
        }
        i += 1;
    }
    None
}

fn next_non_space(bytes: &[u8], from: usize) -> Option<u8> {
    bytes.get(from..)?.iter().copied().find(|b| !b.is_ascii_whitespace())
}

fn previous_non_space(bytes: &[u8], before: usize) -> Option<u8> {
    bytes[..before].iter().rev().copied().find(|b| !b.is_ascii_whitespace())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn highlighter(file: &str) -> Highlighter {
        Highlighter::new(Language::from_path(Path::new(file)).expect("supported language"))
    }

    fn tokens(file: &str, line: &str) -> Vec<(&'static str, TokenKind)> {
        let line: &'static str = Box::leak(line.to_owned().into_boxed_str());
        highlighter(file)
            .highlight_line(line, LineState::Normal)
            .into_iter()
            .map(|t| (&line[t.range], t.kind))
            .collect()
    }

    #[test]
    fn rust_line() {
        assert_eq!(
            tokens("a.rs", r#"pub fn main(s: &str) -> Option<u8> { "hi\"x" } // done"#),
            [
                ("pub", TokenKind::Keyword),
                ("fn", TokenKind::Keyword),
                ("main", TokenKind::Function),
                ("str", TokenKind::Type),
                ("Option", TokenKind::Type),
                ("u8", TokenKind::Type),
                (r#""hi\"x""#, TokenKind::String),
                ("// done", TokenKind::Comment),
            ]
        );
    }

    #[test]
    fn rust_lifetimes_are_not_strings() {
        assert_eq!(
            tokens("a.rs", "fn f<'a>(x: &'a str) -> char { 'x' }"),
            [
                ("fn", TokenKind::Keyword),
                ("str", TokenKind::Type),
                ("char", TokenKind::Type),
                ("'x'", TokenKind::String)
            ]
        );
    }

    #[test]
    fn block_comments_span_lines() {
        let h = highlighter("a.c");
        let lines = ["int a; /* start", "middle", "end */ int b;"];
        let states = h.line_states(lines);
        assert_eq!(states, [LineState::Normal, LineState::BlockComment, LineState::BlockComment]);
        let last = h.highlight_line(lines[2], states[2]);
        assert_eq!(last[0], Token { range: 0..6, kind: TokenKind::Comment });
        assert_eq!(last[1].kind, TokenKind::Keyword);
    }

    #[test]
    fn python_triple_quoted_strings() {
        let h = highlighter("a.py");
        let states = h.line_states(["x = \"\"\"doc", "more", "end\"\"\" + y"]);
        assert_eq!(states[1], LineState::String(0));
        assert_eq!(states[2], LineState::String(0));
    }

    #[test]
    fn yaml_keys_and_comments() {
        assert_eq!(
            tokens("a.yml", "  host: localhost # note"),
            [("host", TokenKind::Key), ("# note", TokenKind::Comment),]
        );
    }

    #[test]
    fn json_keys() {
        assert_eq!(
            tokens("a.json", r#"{"port": 8080, "on": true}"#),
            [
                (r#""port""#, TokenKind::Key),
                ("8080", TokenKind::Number),
                (r#""on""#, TokenKind::Key),
                ("true", TokenKind::Constant),
            ]
        );
    }

    #[test]
    fn handles_multibyte_text() {
        let found = tokens("a.rs", "let é = \"ü\"; // ñ");
        assert_eq!(found.last(), Some(&("// ñ", TokenKind::Comment)));
    }
}
