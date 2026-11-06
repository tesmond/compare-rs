//! Lightweight syntax highlighting for compare-rs.
//!
//! A table-driven lexer recognizes comments, strings, numbers, keywords,
//! constants, types, functions, and keys for common languages. Lexing is
//! line-oriented: [`Highlighter::line_states`] computes the state at the start
//! of every line in one fast pass, after which any single line can be
//! highlighted independently with [`Highlighter::highlight_line`]. That lets
//! views lex only the rows that are on screen.

mod highlighter;
mod language;
mod languages;
mod token;

pub use highlighter::Highlighter;
pub use language::Language;
pub use token::{LineState, Token, TokenKind};
