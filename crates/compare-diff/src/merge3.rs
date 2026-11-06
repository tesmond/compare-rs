//! Three-way merge in the style of `diff3`.

use std::ops::Range;

use crate::diff_options::DiffOptions;
use crate::line_diff;

/// How a [`MergeRegion`] was changed relative to the base.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MergeRegionKind {
    /// Neither side changed these lines.
    Unchanged,
    /// Only the left side changed these lines.
    LeftChanged,
    /// Only the right side changed these lines.
    RightChanged,
    /// Both sides made the identical change.
    BothChanged,
    /// Both sides changed these lines differently.
    Conflict,
}

/// A region of the merge with the corresponding line ranges on each input.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MergeRegion {
    pub kind: MergeRegionKind,
    pub base: Range<usize>,
    pub left: Range<usize>,
    pub right: Range<usize>,
}

impl MergeRegion {
    /// Returns the line range on the side that should be taken automatically,
    /// or `None` for conflicts.
    pub fn auto_resolution(&self) -> Option<AutoSource> {
        match self.kind {
            MergeRegionKind::Unchanged => Some(AutoSource::Base),
            MergeRegionKind::LeftChanged | MergeRegionKind::BothChanged => Some(AutoSource::Left),
            MergeRegionKind::RightChanged => Some(AutoSource::Right),
            MergeRegionKind::Conflict => None,
        }
    }
}

/// The input a non-conflicting region is taken from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AutoSource {
    Base,
    Left,
    Right,
}

/// A change made by one side relative to the base.
#[derive(Clone, Debug)]
struct SideChange {
    base: Range<usize>,
    side: Range<usize>,
}

/// Merges `left` and `right`, which were both derived from `base`.
///
/// The returned regions cover all three inputs completely and in order.
/// Changes from the two sides that overlap or touch in the base form a single
/// region; it is a [`MergeRegionKind::Conflict`] unless both sides produced
/// identical lines.
///
/// # Examples
///
/// ```
/// use compare_diff::{DiffOptions, MergeRegionKind, merge3};
///
/// let base = ["a", "b", "c"];
/// let left = ["A", "b", "c"];
/// let right = ["a", "b", "C"];
/// let kinds: Vec<MergeRegionKind> =
///     merge3(&base, &left, &right, &DiffOptions::default()).iter().map(|r| r.kind).collect();
/// assert_eq!(
///     kinds,
///     [MergeRegionKind::LeftChanged, MergeRegionKind::Unchanged, MergeRegionKind::RightChanged],
/// );
/// ```
pub fn merge3(
    base: &[&str],
    left: &[&str],
    right: &[&str],
    options: &DiffOptions,
) -> Vec<MergeRegion> {
    let left_changes = side_changes(base, left, options);
    let right_changes = side_changes(base, right, options);

    let mut regions: Vec<MergeRegion> = Vec::new();
    let mut base_pos = 0usize;
    // Offsets that map a base line index outside any change to the side.
    let mut left_delta = 0isize;
    let mut right_delta = 0isize;
    let mut li = 0usize;
    let mut ri = 0usize;

    while li < left_changes.len() || ri < right_changes.len() {
        // Seed the cluster with the change that starts first in the base.
        let take_left = match (left_changes.get(li), right_changes.get(ri)) {
            (Some(l), Some(r)) => l.base.start <= r.base.start,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let seed = if take_left { &left_changes[li] } else { &right_changes[ri] };
        let cluster_start = seed.base.start;
        let mut cluster_end = seed.base.end;
        let (left_first, right_first) = (li, ri);
        if take_left {
            li += 1;
        } else {
            ri += 1;
        }

        // Grow the cluster with every change that overlaps or touches it.
        loop {
            if let Some(change) = left_changes.get(li)
                && change.base.start <= cluster_end
            {
                cluster_end = cluster_end.max(change.base.end);
                li += 1;
                continue;
            }
            if let Some(change) = right_changes.get(ri)
                && change.base.start <= cluster_end
            {
                cluster_end = cluster_end.max(change.base.end);
                ri += 1;
                continue;
            }
            break;
        }

        if base_pos < cluster_start {
            regions.push(MergeRegion {
                kind: MergeRegionKind::Unchanged,
                base: base_pos..cluster_start,
                left: shift(base_pos..cluster_start, left_delta),
                right: shift(base_pos..cluster_start, right_delta),
            });
        }

        let cluster = cluster_start..cluster_end;
        let left_range = side_range(&left_changes[left_first..li], &cluster, left_delta);
        let right_range = side_range(&right_changes[right_first..ri], &cluster, right_delta);
        let left_changed = li > left_first;
        let right_changed = ri > right_first;
        let kind = match (left_changed, right_changed) {
            (true, false) => MergeRegionKind::LeftChanged,
            (false, true) => MergeRegionKind::RightChanged,
            _ => {
                let same = left_range.len() == right_range.len()
                    && left[left_range.clone()]
                        .iter()
                        .zip(&right[right_range.clone()])
                        .all(|(a, b)| lines_equal(a, b, options));
                if same { MergeRegionKind::BothChanged } else { MergeRegionKind::Conflict }
            }
        };

        left_delta = left_range.end as isize - cluster_end as isize;
        right_delta = right_range.end as isize - cluster_end as isize;
        regions.push(MergeRegion { kind, base: cluster, left: left_range, right: right_range });
        base_pos = cluster_end;
    }

    if base_pos < base.len() || regions.is_empty() {
        let tail = base_pos..base.len();
        let left_tail = shift(tail.clone(), left_delta);
        let right_tail = shift(tail.clone(), right_delta);
        if !tail.is_empty() || !left_tail.is_empty() || !right_tail.is_empty() {
            regions.push(MergeRegion {
                kind: MergeRegionKind::Unchanged,
                base: tail,
                left: left_tail,
                right: right_tail,
            });
        }
    }
    regions
}

fn side_changes(base: &[&str], side: &[&str], options: &DiffOptions) -> Vec<SideChange> {
    let (base_ids, side_ids) = line_diff::intern_lines(base, side, options);
    let blocks = line_diff::match_ids(&base_ids, &side_ids);
    line_diff::chunks_from_blocks(&blocks, base.len(), side.len())
        .into_iter()
        .filter(|chunk| chunk.is_change())
        .map(|chunk| SideChange { base: chunk.left, side: chunk.right })
        .collect()
}

/// Returns the side's line range corresponding to the base range `cluster`,
/// given the side's changes inside the cluster and the offset before it.
fn side_range(changes: &[SideChange], cluster: &Range<usize>, delta: isize) -> Range<usize> {
    match (changes.first(), changes.last()) {
        (Some(first), Some(last)) => {
            let start = first.side.start - (first.base.start - cluster.start);
            let end = last.side.end + (cluster.end - last.base.end);
            start..end
        }
        _ => shift(cluster.clone(), delta),
    }
}

fn shift(range: Range<usize>, delta: isize) -> Range<usize> {
    (range.start as isize + delta) as usize..(range.end as isize + delta) as usize
}

fn lines_equal(a: &str, b: &str, options: &DiffOptions) -> bool {
    if options.is_exact() {
        return a == b;
    }
    let (a_ids, b_ids) = line_diff::intern_lines(&[a], &[b], options);
    a_ids == b_ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge(base: &[&str], left: &[&str], right: &[&str]) -> Vec<MergeRegion> {
        merge3(base, left, right, &DiffOptions::default())
    }

    /// Checks that regions tile all three inputs without gaps.
    fn assert_tiles(regions: &[MergeRegion], base: usize, left: usize, right: usize) {
        let mut positions = (0, 0, 0);
        for region in regions {
            assert_eq!(region.base.start, positions.0, "base gap in {regions:?}");
            assert_eq!(region.left.start, positions.1, "left gap in {regions:?}");
            assert_eq!(region.right.start, positions.2, "right gap in {regions:?}");
            positions = (region.base.end, region.left.end, region.right.end);
        }
        assert_eq!(positions, (base, left, right));
    }

    #[test]
    fn no_changes() {
        let regions = merge(&["a", "b"], &["a", "b"], &["a", "b"]);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].kind, MergeRegionKind::Unchanged);
    }

    #[test]
    fn all_empty() {
        assert!(merge(&[], &[], &[]).is_empty());
    }

    #[test]
    fn conflict_when_both_edit_same_line() {
        let base = ["a", "b", "c"];
        let left = ["a", "L", "c"];
        let right = ["a", "R", "c"];
        let regions = merge(&base, &left, &right);
        assert_tiles(&regions, 3, 3, 3);
        let kinds: Vec<_> = regions.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [MergeRegionKind::Unchanged, MergeRegionKind::Conflict, MergeRegionKind::Unchanged]
        );
    }

    #[test]
    fn identical_change_on_both_sides() {
        let regions = merge(&["a", "b"], &["a", "X"], &["a", "X"]);
        assert_eq!(regions.last().map(|r| r.kind), Some(MergeRegionKind::BothChanged));
    }

    #[test]
    fn insertions_and_deletions_shift_ranges() {
        let base = ["1", "2", "3", "4", "5"];
        let left = ["0", "1", "2", "3", "4", "5"];
        let right = ["1", "2", "3", "5"];
        let regions = merge(&base, &left, &right);
        assert_tiles(&regions, 5, 6, 4);
        let kinds: Vec<_> = regions.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [
                MergeRegionKind::LeftChanged,
                MergeRegionKind::Unchanged,
                MergeRegionKind::RightChanged,
                MergeRegionKind::Unchanged,
            ]
        );
    }

    #[test]
    fn appends_on_both_sides_conflict() {
        let regions = merge(&["a"], &["a", "L"], &["a", "R"]);
        assert_tiles(&regions, 1, 2, 2);
        assert_eq!(regions.last().map(|r| r.kind), Some(MergeRegionKind::Conflict));
    }
}
