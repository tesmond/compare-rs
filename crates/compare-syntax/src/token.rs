//! Tokens produced by the lexer.

use std::ops::Range;

/// The syntactic category of a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Comment,
    String,
    Number,
    Keyword,
    /// Literal constants such as `true`, `null`, or `None`.
    Constant,
    /// Capitalized identifiers in languages where those name types.
    Type,
    /// An identifier immediately followed by `(`.
    Function,
    /// A key in a key/value file (YAML, TOML, JSON) or an XML/HTML tag name.
    Key,
    /// Preprocessor directives, attributes, and decorators.
    Meta,
}

/// A highlighted byte range within a line.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// Lexer state carried from the end of one line to the start of the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineState {
    #[default]
    Normal,
    /// Inside a block comment.
    BlockComment,
    /// Inside a multi-line string; the value indexes the language's string
    /// delimiters.
    String(u8),
}
