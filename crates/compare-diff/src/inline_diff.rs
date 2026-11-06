//! Highlighting of changed spans within a pair of lines.

use std::ops::Range;

use crate::myers;

/// Byte ranges that differ within a pair of lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InlineDiff {
    /// Changed byte ranges in the left line, sorted and non-overlapping.
    pub left: Vec<Range<usize>>,
    /// Changed byte ranges in the right line, sorted and non-overlapping.
    pub right: Vec<Range<usize>>,
}

/// Compares two lines word-by-word and returns the changed byte ranges.
///
/// Words are runs of alphanumeric characters or `_`; every other character is
/// its own token. Diffing tokens instead of characters avoids highlighting
/// scattered coincidental letters inside replaced words.
///
/// # Examples
///
/// ```
/// let inline = compare_diff::diff_inline("let x = 1;", "let y = 1;");
/// assert_eq!(inline.left, vec![4..5]);
/// assert_eq!(inline.right, vec![4..5]);
/// ```
pub fn diff_inline(left: &str, right: &str) -> InlineDiff {
    if left == right {
        return InlineDiff::default();
    }
    let left_tokens = tokenize(left);
    let right_tokens = tokenize(right);
    let left_text: Vec<&str> = left_tokens.iter().map(|r| &left[r.clone()]).collect();
    let right_text: Vec<&str> = right_tokens.iter().map(|r| &right[r.clone()]).collect();
    let blocks = myers::diff(&left_text, &right_text);

    let mut inline = InlineDiff::default();
    let mut left_pos = 0;
    let mut right_pos = 0;
    for block in &blocks {
        push_span(&mut inline.left, &left_tokens, left_pos..block.left_start);
        push_span(&mut inline.right, &right_tokens, right_pos..block.right_start);
        left_pos = block.left_start + block.len;
        right_pos = block.right_start + block.len;
    }
    push_span(&mut inline.left, &left_tokens, left_pos..left_tokens.len());
    push_span(&mut inline.right, &right_tokens, right_pos..right_tokens.len());
    inline
}

/// Appends the byte span covered by `tokens[token_range]`, merging with the
/// previous span when they touch.
fn push_span(spans: &mut Vec<Range<usize>>, tokens: &[Range<usize>], token_range: Range<usize>) {
    if token_range.is_empty() {
        return;
    }
    let span = tokens[token_range.start].start..tokens[token_range.end - 1].end;
    match spans.last_mut() {
        Some(last) if last.end == span.start => last.end = span.end,
        _ => spans.push(span),
    }
}

fn tokenize(text: &str) -> Vec<Range<usize>> {
    let mut tokens: Vec<Range<usize>> = Vec::new();
    let mut word_start: Option<usize> = None;
    for (index, ch) in text.char_indices() {
        if ch.is_alphanumeric() || ch == '_' {
            word_start.get_or_insert(index);
            continue;
        }
        if let Some(start) = word_start.take() {
            tokens.push(start..index);
        }
        tokens.push(index..index + ch.len_utf8());
    }
    if let Some(start) = word_start {
        tokens.push(start..text.len());
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_lines_have_no_spans() {
        assert_eq!(diff_inline("same", "same"), InlineDiff::default());
    }

    #[test]
    fn whole_words_are_highlighted() {
        let inline = diff_inline("timeout: 30", "timeout: 60");
        assert_eq!(inline.left, vec![9..11]);
        assert_eq!(inline.right, vec![9..11]);
    }

    #[test]
    fn insertion_only_on_right() {
        let inline = diff_inline("a b", "a new b");
        assert!(inline.left.is_empty());
        assert_eq!(inline.right, vec![2..6]);
    }

    #[test]
    fn multibyte_characters_use_byte_offsets() {
        let inline = diff_inline("héllo wörld", "héllo world");
        assert_eq!(inline.left, vec![7..13]);
        assert_eq!(inline.right, vec![7..12]);
    }
}
