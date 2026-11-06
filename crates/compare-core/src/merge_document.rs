//! A three-way merge with user-selected conflict resolutions.

use compare_diff::{AutoSource, DiffOptions, MergeRegion, MergeRegionKind};

use crate::text_document::TextDocument;

/// How a merge region contributes to the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    /// Not yet decided; written out with conflict markers.
    Unresolved,
    Base,
    Left,
    Right,
    LeftThenRight,
    RightThenLeft,
}

impl From<AutoSource> for Resolution {
    fn from(source: AutoSource) -> Self {
        match source {
            AutoSource::Base => Resolution::Base,
            AutoSource::Left => Resolution::Left,
            AutoSource::Right => Resolution::Right,
        }
    }
}

/// One row of the three-column merge view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MergeRow {
    /// Index into [`MergeDocument::regions`].
    pub region: u32,
    pub left: Option<u32>,
    pub base: Option<u32>,
    pub right: Option<u32>,
}

/// Where a line of merged output comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputLine {
    Left(u32),
    Base(u32),
    Right(u32),
    /// `<<<<<<<` marker opening an unresolved conflict.
    ConflictStart,
    /// `=======` marker between the two sides of an unresolved conflict.
    ConflictSeparator,
    /// `>>>>>>>` marker closing an unresolved conflict.
    ConflictEnd,
}

/// A three-way merge of `left` and `right` against their common `base`.
#[derive(Clone, Debug)]
pub struct MergeDocument {
    pub left: TextDocument,
    pub base: TextDocument,
    pub right: TextDocument,
    pub regions: Vec<MergeRegion>,
    /// Resolution for each region; non-conflicting regions start resolved.
    pub resolutions: Vec<Resolution>,
    pub rows: Vec<MergeRow>,
    /// Indices of conflicting regions, in order.
    pub conflicts: Vec<usize>,
    /// For each region, the index of its first row.
    pub region_first_row: Vec<usize>,
}

impl MergeDocument {
    pub fn new(
        left: TextDocument,
        base: TextDocument,
        right: TextDocument,
        options: &DiffOptions,
    ) -> Self {
        let regions = compare_diff::merge3(&base.lines(), &left.lines(), &right.lines(), options);
        let resolutions: Vec<Resolution> = regions
            .iter()
            .map(|region| region.auto_resolution().map_or(Resolution::Unresolved, Resolution::from))
            .collect();
        let conflicts: Vec<usize> = regions
            .iter()
            .enumerate()
            .filter(|(_, region)| region.kind == MergeRegionKind::Conflict)
            .map(|(index, _)| index)
            .collect();

        let mut rows: Vec<MergeRow> = Vec::new();
        let mut region_first_row: Vec<usize> = Vec::with_capacity(regions.len());
        for (index, region) in regions.iter().enumerate() {
            region_first_row.push(rows.len());
            let height = region.left.len().max(region.base.len()).max(region.right.len());
            let line = |range: &std::ops::Range<usize>, offset: usize| {
                (offset < range.len()).then(|| (range.start + offset) as u32)
            };
            for offset in 0..height {
                rows.push(MergeRow {
                    region: index as u32,
                    left: line(&region.left, offset),
                    base: line(&region.base, offset),
                    right: line(&region.right, offset),
                });
            }
        }

        Self { left, base, right, regions, resolutions, rows, conflicts, region_first_row }
    }

    /// Number of conflicts that still need a decision.
    pub fn unresolved_count(&self) -> usize {
        self.resolutions.iter().filter(|r| **r == Resolution::Unresolved).count()
    }

    /// Lists the merged output, line by line.
    pub fn output_lines(&self) -> Vec<OutputLine> {
        let mut output: Vec<OutputLine> = Vec::with_capacity(self.base.len());
        for (region, resolution) in self.regions.iter().zip(&self.resolutions) {
            let left = region.left.clone().map(|i| OutputLine::Left(i as u32));
            let base = region.base.clone().map(|i| OutputLine::Base(i as u32));
            let right = region.right.clone().map(|i| OutputLine::Right(i as u32));
            match resolution {
                Resolution::Base => output.extend(base),
                Resolution::Left => output.extend(left),
                Resolution::Right => output.extend(right),
                Resolution::LeftThenRight => output.extend(left.chain(right)),
                Resolution::RightThenLeft => output.extend(right.chain(left)),
                Resolution::Unresolved => {
                    output.push(OutputLine::ConflictStart);
                    output.extend(left);
                    output.push(OutputLine::ConflictSeparator);
                    output.extend(right);
                    output.push(OutputLine::ConflictEnd);
                }
            }
        }
        output
    }

    /// Text of one output line.
    pub fn output_text(&self, line: OutputLine) -> &str {
        match line {
            OutputLine::Left(i) => self.left.line(i as usize),
            OutputLine::Base(i) => self.base.line(i as usize),
            OutputLine::Right(i) => self.right.line(i as usize),
            OutputLine::ConflictStart => "<<<<<<< left",
            OutputLine::ConflictSeparator => "=======",
            OutputLine::ConflictEnd => ">>>>>>> right",
        }
    }

    /// The complete merged text, using the base file's line endings.
    pub fn merged_text(&self) -> String {
        let newline = if self.base.text().contains("\r\n") { "\r\n" } else { "\n" };
        let mut text = String::with_capacity(self.base.text().len());
        for line in self.output_lines() {
            text.push_str(self.output_text(line));
            text.push_str(newline);
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> TextDocument {
        TextDocument::new(text.to_owned())
    }

    #[test]
    fn merges_non_conflicting_changes() {
        let merge = MergeDocument::new(
            doc("A\nb\nc\n"),
            doc("a\nb\nc\n"),
            doc("a\nb\nC\n"),
            &DiffOptions::default(),
        );
        assert_eq!(merge.unresolved_count(), 0);
        assert_eq!(merge.merged_text(), "A\nb\nC\n");
    }

    #[test]
    fn conflicts_need_resolution() {
        let mut merge = MergeDocument::new(
            doc("a\nL\nc\n"),
            doc("a\nb\nc\n"),
            doc("a\nR\nc\n"),
            &DiffOptions::default(),
        );
        assert_eq!(merge.conflicts.len(), 1);
        assert_eq!(merge.merged_text(), "a\n<<<<<<< left\nL\n=======\nR\n>>>>>>> right\nc\n");

        let conflict = merge.conflicts[0];
        merge.resolutions[conflict] = Resolution::RightThenLeft;
        assert_eq!(merge.unresolved_count(), 0);
        assert_eq!(merge.merged_text(), "a\nR\nL\nc\n");
    }
}
