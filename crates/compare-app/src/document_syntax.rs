//! Syntax state for one document shown in a view.

use std::path::Path;

use compare_core::{LineChange, TextDocument};
use compare_syntax::{Highlighter, Language, LineState, Token};

/// Highlights lines of one document on demand.
///
/// The lexer state at the start of each line is computed up front (a fast
/// pass), so any visible line can be highlighted independently. After an
/// edit only the states from the edit onward are recomputed, stopping as
/// soon as they match the old ones.
#[derive(Clone, Debug, Default)]
pub struct DocumentSyntax {
    language: Option<Language>,
    highlighter: Option<Highlighter>,
    line_states: Vec<LineState>,
}

impl DocumentSyntax {
    /// Detects the language from `path`; unknown languages are left plain.
    pub fn new(path: &Path, document: &TextDocument) -> Self {
        let Some(language) = Language::from_path(path) else {
            return Self::default();
        };
        let highlighter = Highlighter::new(language);
        let line_states = highlighter.line_states((0..document.len()).map(|i| document.line(i)));
        Self { language: Some(language), highlighter: Some(highlighter), line_states }
    }

    /// Tokens for line `index`, whose display text is `text`.
    pub fn tokens(&self, index: usize, text: &str) -> Vec<Token> {
        match (&self.highlighter, self.line_states.get(index)) {
            (Some(highlighter), Some(&state)) => highlighter.highlight_line(text, state),
            _ => Vec::new(),
        }
    }

    /// Name of the detected language, if any.
    pub fn language_name(&self) -> Option<&'static str> {
        self.language.map(|language| language.name())
    }

    /// Updates the line states after `change`; `lines` is the edited text.
    pub fn apply_edit(&mut self, change: LineChange, lines: &[String]) {
        let Some(highlighter) = self.highlighter else {
            return;
        };
        // The state at the start of the first edited line is unchanged.
        let start = change.first_line;
        if start < self.line_states.len() {
            let old_end = (start + change.old_count).min(self.line_states.len());
            self.line_states.splice(
                start + 1..old_end.max(start + 1),
                std::iter::repeat_n(LineState::Normal, change.new_count - 1),
            );
        }
        self.recompute_from(highlighter, start, start + change.new_count, lines);
    }

    /// Recomputes states after line `start`, at least up to `must_reach`,
    /// then until they agree with the stored ones.
    fn recompute_from(
        &mut self,
        highlighter: Highlighter,
        start: usize,
        must_reach: usize,
        lines: &[String],
    ) {
        self.line_states.resize(lines.len(), LineState::Normal);
        let mut state = self.line_states.get(start).copied().unwrap_or_default();
        for (index, line) in lines.iter().enumerate().skip(start) {
            if index > start {
                if index >= must_reach && self.line_states[index] == state {
                    return;
                }
                self.line_states[index] = state;
            }
            state = highlighter.end_state(line, state);
        }
    }

    /// Makes sure every line of `lines` has a state, for text whose last
    /// empty line was not part of the original document.
    pub fn cover(&mut self, lines: &[String]) {
        if let Some(highlighter) = self.highlighter
            && self.line_states.len() < lines.len()
        {
            let start = self.line_states.len().saturating_sub(1);
            self.recompute_from(highlighter, start, lines.len(), lines);
        }
    }
}

#[cfg(test)]
mod tests {
    use compare_core::{EditBuffer, Encoding, Position};
    use compare_syntax::TokenKind;

    use super::*;

    #[test]
    fn opening_a_block_comment_recolors_later_lines() {
        let text = "let a = 1;\nlet b = 2;\nlet c = 3;\n";
        let mut buffer = EditBuffer::from_text(text, Encoding::Utf8);
        let mut syntax = DocumentSyntax::new(Path::new("x.rs"), &TextDocument::new(text.into()));
        syntax.cover(buffer.lines());
        let change = buffer.replace(Position::new(0, 0)..Position::new(0, 0), "/*");
        syntax.apply_edit(change, buffer.lines());
        let kinds: Vec<TokenKind> =
            syntax.tokens(2, buffer.line(2)).iter().map(|t| t.kind).collect();
        assert_eq!(kinds, [TokenKind::Comment]);

        let change = buffer.replace(Position::new(0, 0)..Position::new(0, 2), "");
        syntax.apply_edit(change, buffer.lines());
        assert_eq!(syntax.tokens(2, buffer.line(2))[0].kind, TokenKind::Keyword);
    }
}
