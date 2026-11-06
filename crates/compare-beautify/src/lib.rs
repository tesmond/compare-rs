//! Reformats compact machine output so it diffs line by line.
//!
//! [`beautify`] spreads JSON, Python `repr` output (dicts, lists, tuples,
//! `Name(...)` objects), and similar bracketed data over multiple indented
//! lines, and turns literal `\n` escape sequences into real line breaks. It is
//! deliberately tolerant: it never parses values, so single quotes, `None`,
//! `True`, trailing commas, and other non-JSON syntax are all fine.

mod structure;
mod unescape;

use std::borrow::Cow;

/// Beautifies `text`, or returns it unchanged if there is nothing to do.
///
/// - If the whole text is one bracketed structure, it is reformatted with one
///   element per line.
/// - Otherwise literal `\n`, `\t`, and `\r` escapes are expanded, and any line
///   that ends in a bracketed structure (such as a log line followed by a
///   JSON payload) has that structure reformatted.
///
/// # Examples
///
/// ```
/// let pretty = compare_beautify::beautify(r#"{"a": 1, "b": [true, null]}"#);
/// assert_eq!(pretty, "{\n  \"a\": 1,\n  \"b\": [\n    true,\n    null\n  ]\n}");
///
/// let python = compare_beautify::beautify("{'k': None, 'v': (1, 2)}");
/// assert_eq!(python, "{\n  'k': None,\n  'v': (\n    1,\n    2\n  )\n}");
///
/// assert_eq!(compare_beautify::beautify(r"one\ntwo"), "one\ntwo");
/// ```
pub fn beautify(text: &str) -> Cow<'_, str> {
    let trimmed = text.trim();
    if structure::is_whole_structure(trimmed) {
        return Cow::Owned(structure::format(trimmed, ""));
    }

    let text: Cow<'_, str> = if unescape::has_escapes(text) {
        Cow::Owned(unescape::unescape(text))
    } else {
        Cow::Borrowed(text)
    };

    let mut changed = matches!(text, Cow::Owned(_));
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        match structure::trailing_structure(line) {
            Some(start) => {
                let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                out.push_str(&line[..start]);
                out.push_str(&structure::format(line[start..].trim_end(), &indent));
                changed = true;
            }
            None => out.push_str(line),
        }
    }
    if changed { Cow::Owned(out) } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_plain_text_alone() {
        let text = "fn main() {\n    println!(\"hi\");\n}\n";
        assert!(matches!(beautify(text), Cow::Borrowed(_)));
    }

    #[test]
    fn formats_json_lines_in_logs() {
        let text = "INFO request {\"id\": 7, \"tags\": [\"a\"]}\nDONE";
        assert_eq!(
            beautify(text),
            "INFO request {\n  \"id\": 7,\n  \"tags\": [\n    \"a\"\n  ]\n}\nDONE"
        );
    }

    #[test]
    fn keeps_line_indentation_for_nested_structures() {
        assert_eq!(beautify("    data = [1, 2]"), "    data = [\n      1,\n      2\n    ]");
    }

    #[test]
    fn expands_escaped_newlines_then_formats() {
        assert_eq!(
            beautify(r#""Traceback:\n  line 1\n{'a': 1}""#),
            "Traceback:\n  line 1\n{\n  'a': 1\n}"
        );
    }

    #[test]
    fn is_idempotent() {
        let once = beautify(r#"{"a": {"b": [1, 2, {}]}, "c": "x, y"}"#).into_owned();
        assert_eq!(beautify(&once), once);
    }
}
