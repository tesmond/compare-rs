//! Options that control how lines are considered equal.

/// Controls which differences are ignored when matching lines.
///
/// Ignored differences still appear in the text; they only stop lines from
/// being reported as changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DiffOptions {
    /// Ignore all whitespace when comparing lines, like `diff -w`.
    pub ignore_whitespace: bool,
    /// Compare lines case-insensitively.
    pub ignore_case: bool,
}

impl DiffOptions {
    /// Returns true if lines can be compared byte-for-byte.
    pub fn is_exact(&self) -> bool {
        !self.ignore_whitespace && !self.ignore_case
    }
}
