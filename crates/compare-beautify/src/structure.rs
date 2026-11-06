//! Tolerant pretty-printing of bracketed data.

const INDENT: &str = "  ";

fn closer(open: char) -> Option<char> {
    match open {
        '{' => Some('}'),
        '[' => Some(']'),
        '(' => Some(')'),
        _ => None,
    }
}

fn is_closer(c: char) -> bool {
    matches!(c, '}' | ']' | ')')
}

/// Returns the byte offset just past the bracket that closes the one at
/// `start`, or `None` if the brackets do not balance.
fn structure_end(text: &str, start: usize) -> Option<usize> {
    let mut stack: Vec<char> = Vec::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, c) in text[start..].char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '{' | '[' | '(' => stack.extend(closer(c)),
            '}' | ']' | ')' => {
                if stack.pop() != Some(c) {
                    return None;
                }
                if stack.is_empty() {
                    return Some(start + index + c.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// Returns true if a bracketed region contains a separator (`,` or `:`) or a
/// nested bracket outside strings, i.e. formatting it would split it up.
fn has_inner_structure(text: &str) -> bool {
    let inner = &text[1..text.len() - 1];
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in inner.chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            ',' | ':' | '{' | '[' | '(' => return true,
            _ => {}
        }
    }
    false
}

/// Returns true if `text` (already trimmed) is a single bracketed structure,
/// optionally preceded by a constructor name as in Python's `Point(x=1)` or
/// `OrderedDict([...])`.
pub(crate) fn is_whole_structure(text: &str) -> bool {
    let Some(start) = text.find(['{', '[', '(']) else {
        return false;
    };
    let prefix = &text[..start];
    let prefix_ok = prefix.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && (prefix.is_empty() || text[start..].starts_with('('));
    prefix_ok
        && structure_end(text, start) == Some(text.len())
        && has_inner_structure(&text[start..])
}

/// If `line` ends with a `{...}` or `[...]` structure worth formatting,
/// returns the byte offset where it starts.
pub(crate) fn trailing_structure(line: &str) -> Option<usize> {
    let end = line.trim_end().len();
    line.char_indices().filter(|(_, c)| *c == '{' || *c == '[').map(|(index, _)| index).find(
        |&start| structure_end(line, start) == Some(end) && has_inner_structure(&line[start..end]),
    )
}

/// Pretty-prints `text`, putting each element on its own line indented by
/// [`INDENT`] per nesting level, on top of `base_indent`.
pub(crate) fn format(text: &str, base_indent: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let next_non_space = |from: usize| {
        (from..chars.len()).find(|&j| !chars[j].is_whitespace()).unwrap_or(chars.len())
    };
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.push_str(base_indent);
        for _ in 0..depth {
            out.push_str(INDENT);
        }
    };

    let mut out = String::with_capacity(text.len() * 2);
    let mut stack: Vec<char> = Vec::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                out.push(c);
            }
            '{' | '[' | '(' => {
                let next = next_non_space(i + 1);
                out.push(c);
                if chars.get(next).copied() == closer(c) {
                    // Keep empty containers inline: `{}`, `[]`, `()`.
                    out.extend(closer(c));
                    i = next + 1;
                    continue;
                }
                stack.push(c);
                newline(&mut out, stack.len());
            }
            '}' | ']' | ')' => {
                stack.pop();
                // Drop the blank line a trailing comma would leave behind.
                let trimmed_len = out.trim_end_matches([' ', '\t']).len();
                out.truncate(trimmed_len);
                if out.ends_with('\n') {
                    out.pop();
                }
                newline(&mut out, stack.len());
                out.push(c);
            }
            ',' if !stack.is_empty() => {
                out.push(',');
                newline(&mut out, stack.len());
                i = next_non_space(i + 1);
                continue;
            }
            ':' if stack.last() == Some(&'{') => {
                out.push_str(": ");
                i = next_non_space(i + 1);
                continue;
            }
            c if c.is_whitespace() => {
                let next = next_non_space(i);
                let after_break = out.is_empty() || out.ends_with(['\n', ' ', '\t']);
                let before_punctuation =
                    chars.get(next).is_none_or(|&n| is_closer(n) || n == ',' || n == ':');
                if !after_break && !before_punctuation {
                    out.push(' ');
                }
                i = next;
                continue;
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_structures() {
        assert!(is_whole_structure(r#"{"a": 1}"#));
        assert!(is_whole_structure("Point(x=1, y=2)"));
        assert!(is_whole_structure("[[1], [2]]"));
        assert!(!is_whole_structure("[x]"));
        assert!(!is_whole_structure("{\"a\": 1} trailing"));
        assert!(!is_whole_structure("call(a, b); other(c)"));
        assert!(!is_whole_structure(r#"{"a": "}"#));
    }

    #[test]
    fn brackets_inside_strings_are_ignored() {
        assert_eq!(format(r#"{"k": "a, [b]: c"}"#, ""), "{\n  \"k\": \"a, [b]: c\"\n}");
    }

    #[test]
    fn trailing_commas_leave_no_blank_lines() {
        assert_eq!(format("[1, 2,]", ""), "[\n  1,\n  2,\n]");
    }

    #[test]
    fn python_objects() {
        assert_eq!(
            format("User(name='Ann', roles=['a', 'b'], meta={})", ""),
            "User(\n  name='Ann',\n  roles=[\n    'a',\n    'b'\n  ],\n  meta={}\n)"
        );
    }

    #[test]
    fn finds_trailing_structure_in_log_lines() {
        assert_eq!(trailing_structure("got payload={\"a\": 1}"), Some(12));
        assert_eq!(trailing_structure("fn main() {"), None);
        assert_eq!(trailing_structure("let v = [1, 2];"), None);
    }
}
