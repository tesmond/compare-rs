//! Expanding escape sequences in text copied from string literals or logs.

/// Returns true if `text` contains literal `\n` or `\r\n` escape sequences.
pub(crate) fn has_escapes(text: &str) -> bool {
    text.contains("\\n")
}

/// Expands `\n`, `\r`, `\t`, `\\`, `\"`, and `\'`. Other escapes are kept as
/// written. Quotes wrapping the whole text (as when a string literal is
/// copied with them) are removed.
pub(crate) fn unescape(text: &str) -> String {
    let trimmed = text.trim();
    let inner = ['"', '\'']
        .iter()
        .find_map(|&quote| trimmed.strip_prefix(quote).and_then(|rest| rest.strip_suffix(quote)))
        .unwrap_or(text);

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => {}
            Some('t') => out.push('\t'),
            Some(escaped @ ('\\' | '"' | '\'')) => out.push(escaped),
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

    #[test]
    fn expands_common_escapes() {
        assert_eq!(unescape(r"a\nb\tc\r\nd \u00e9"), "a\nb\tc\nd \\u00e9");
    }

    #[test]
    fn strips_wrapping_quotes() {
        assert_eq!(unescape(r#"  "x\n\"y\""  "#), "x\n\"y\"");
        assert_eq!(unescape(r"'it\'s\nok'"), "it's\nok");
    }

    #[test]
    fn escaped_backslash_is_not_a_newline() {
        assert_eq!(unescape(r"C:\\new"), r"C:\new");
    }
}
