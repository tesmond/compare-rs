//! Finding (and replacing) text across lines.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

use crate::edit_buffer::Position;

/// How a search pattern is interpreted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    /// Treat the pattern as a regular expression rather than literal text.
    pub regex: bool,
}

/// A compiled search pattern. Patterns may span lines: a line break in a
/// literal pattern (or `\n` in a regular expression) matches the end of a
/// line.
#[derive(Clone, Debug)]
pub struct SearchQuery {
    regex: Regex,
    options: SearchOptions,
}

/// Lines joined by `\n`, with the byte offset where each line starts.
struct JoinedText {
    text: String,
    line_starts: Vec<usize>,
}

impl JoinedText {
    fn new(lines: &[String]) -> Self {
        let mut line_starts = Vec::with_capacity(lines.len());
        let mut offset = 0;
        for line in lines {
            line_starts.push(offset);
            offset += line.len() + 1;
        }
        Self { text: lines.join("\n"), line_starts }
    }

    fn position(&self, offset: usize) -> Position {
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        Position::new(line, offset - self.line_starts[line])
    }
}

impl SearchQuery {
    /// Compiles `pattern`. Fails only for an invalid regular expression.
    pub fn new(pattern: &str, options: SearchOptions) -> Result<Self, regex::Error> {
        let mut source = if options.regex { pattern.to_owned() } else { regex::escape(pattern) };
        if options.whole_word {
            source = format!(r"\b(?:{source})\b");
        }
        let regex = RegexBuilder::new(&source)
            .case_insensitive(!options.match_case)
            .multi_line(true)
            .build()?;
        Ok(Self { regex, options })
    }

    /// Every non-empty match in `lines`, in order.
    pub fn find_all(&self, lines: &[String]) -> Vec<Range<Position>> {
        let joined = JoinedText::new(lines);
        self.regex
            .find_iter(&joined.text)
            .filter(|found| !found.is_empty())
            .map(|found| joined.position(found.start())..joined.position(found.end()))
            .collect()
    }

    /// Every match in `lines` paired with its replacement text.
    ///
    /// In regular-expression mode, `$1`, `${name}`, and the escapes `\n`,
    /// `\t`, and `\\` are expanded; otherwise `replacement` is used as is.
    pub fn replacements(
        &self,
        lines: &[String],
        replacement: &str,
    ) -> Vec<(Range<Position>, String)> {
        let joined = JoinedText::new(lines);
        let template = if self.options.regex { unescape(replacement) } else { String::new() };
        self.regex
            .captures_iter(&joined.text)
            .filter_map(|captures| {
                let found = captures.get(0)?;
                if found.is_empty() {
                    return None;
                }
                let text = if self.options.regex {
                    let mut expanded = String::new();
                    captures.expand(&template, &mut expanded);
                    expanded
                } else {
                    replacement.to_owned()
                };
                Some((joined.position(found.start())..joined.position(found.end()), text))
            })
            .collect()
    }
}

/// Expands `\n`, `\t`, and `\\` in a regular-expression replacement.
fn unescape(replacement: &str) -> String {
    let mut out = String::with_capacity(replacement.len());
    let mut chars = replacement.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.split('\n').map(str::to_owned).collect()
    }

    fn pos(line: usize, column: usize) -> Position {
        Position::new(line, column)
    }

    #[test]
    fn literal_search_ignores_case_by_default() {
        let query = SearchQuery::new("a.b", SearchOptions::default()).expect("valid");
        assert_eq!(
            query.find_all(&lines("A.B axb\nx a.b")),
            [pos(0, 0)..pos(0, 3), pos(1, 2)..pos(1, 5)]
        );
        let exact = SearchOptions { match_case: true, ..Default::default() };
        let query = SearchQuery::new("a.b", exact).expect("valid");
        assert_eq!(query.find_all(&lines("A.B a.b")), [pos(0, 4)..pos(0, 7)]);
    }

    #[test]
    fn whole_words_only() {
        let options = SearchOptions { whole_word: true, ..Default::default() };
        let query = SearchQuery::new("cat", options).expect("valid");
        assert_eq!(query.find_all(&lines("cat concat cat_x cat")).len(), 2);
    }

    #[test]
    fn patterns_span_lines() {
        let query = SearchQuery::new("end\nstart", SearchOptions::default()).expect("valid");
        assert_eq!(query.find_all(&lines("the end\nstart here")), [pos(0, 4)..pos(1, 5)]);
    }

    #[test]
    fn regex_replacements_expand_groups_and_newlines() {
        let options = SearchOptions { regex: true, ..Default::default() };
        let query = SearchQuery::new(r"(\w+)=(\w+)", options).expect("valid");
        let found = query.replacements(&lines("a=1, b=2"), r"$1:\n  $2");
        let texts: Vec<&str> = found.iter().map(|(_, text)| text.as_str()).collect();
        assert_eq!(texts, ["a:\n  1", "b:\n  2"]);
        assert!(SearchQuery::new("(", options).is_err());
    }

    #[test]
    fn literal_replacements_are_verbatim() {
        let query = SearchQuery::new("x", SearchOptions::default()).expect("valid");
        let found = query.replacements(&lines("x"), "$1\\n");
        assert_eq!(found[0].1, "$1\\n");
    }

    #[test]
    fn empty_regex_matches_are_skipped() {
        let options = SearchOptions { regex: true, ..Default::default() };
        let query = SearchQuery::new("^", options).expect("valid");
        assert!(query.find_all(&lines("a\nb")).is_empty());
    }
}
