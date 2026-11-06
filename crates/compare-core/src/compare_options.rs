//! Options for comparing text.

use compare_diff::DiffOptions;

/// Everything that affects how two texts are compared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CompareOptions {
    /// Which differences count when matching lines.
    pub diff: DiffOptions,
    /// Reformat JSON, Python dicts, and escaped `\n` text onto multiple lines
    /// before comparing (see [`compare_beautify::beautify`]).
    pub beautify: bool,
}
