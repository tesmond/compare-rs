//! Language definitions and detection.

use std::path::Path;

use crate::languages::LANGUAGES;

/// How a string literal is delimited.
#[derive(Debug)]
pub(crate) struct StringDelimiter {
    pub open: &'static str,
    pub close: &'static str,
    /// Whether the string may continue onto following lines.
    pub multiline: bool,
    /// Whether `\` escapes the next character.
    pub escapes: bool,
    /// The literal holds exactly one character or one escape sequence, as in
    /// C or Rust `'a'`; anything else (such as the Rust lifetime `'a`) is not
    /// a string.
    pub char_literal: bool,
}

/// Lexical rules for one language.
#[derive(Debug)]
pub(crate) struct LanguageSpec {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub file_names: &'static [&'static str],
    pub line_comments: &'static [&'static str],
    pub block_comment: Option<(&'static str, &'static str)>,
    /// Ordered so longer delimiters are tried first.
    pub strings: &'static [StringDelimiter],
    pub keywords: &'static [&'static str],
    pub constants: &'static [&'static str],
    /// Built-in type names, such as `i32` or `string`.
    pub types: &'static [&'static str],
    pub case_insensitive_keywords: bool,
    /// Capitalized identifiers are highlighted as types.
    pub capitalized_types: bool,
    /// `key:` / `key =` at the start of a line, and strings followed by `:`,
    /// are highlighted as keys.
    pub keys: bool,
    /// Names after `<` or `</` are highlighted as tags.
    pub tags: bool,
    /// Lines starting with this prefix (after indentation) are meta lines,
    /// e.g. `#` for the C preprocessor or `[` for INI sections.
    pub meta_line_prefix: Option<&'static str>,
    /// Characters that start a meta token such as `@decorator` or `#[attr]`.
    pub meta_prefix: Option<char>,
}

/// A language the highlighter understands.
#[derive(Clone, Copy, Debug)]
pub struct Language {
    pub(crate) spec: &'static LanguageSpec,
}

impl Language {
    /// Detects the language from a file's name or extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        let file_name = path.file_name()?.to_str()?;
        let lower = file_name.to_ascii_lowercase();
        let extension = Path::new(&lower).extension().and_then(|e| e.to_str()).unwrap_or("");
        LANGUAGES
            .iter()
            .find(|spec| {
                spec.file_names.contains(&file_name) || spec.extensions.contains(&extension)
            })
            .map(|spec| Self { spec })
    }

    /// Human-readable language name, e.g. `Rust`.
    pub fn name(&self) -> &'static str {
        self.spec.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_languages() {
        let name = |path: &str| Language::from_path(Path::new(path)).map(|l| l.name());
        assert_eq!(name("src/main.rs"), Some("Rust"));
        assert_eq!(name("App.TSX"), Some("TypeScript"));
        assert_eq!(name("Makefile"), Some("Makefile"));
        assert_eq!(name("config.yml"), Some("YAML"));
        assert_eq!(name("notes.unknown"), None);
    }
}
