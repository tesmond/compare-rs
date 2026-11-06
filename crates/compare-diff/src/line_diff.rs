//! Line-level diff.

use std::borrow::Cow;

use rustc_hash::FxHashMap;

use crate::diff_chunk::{ChunkKind, DiffChunk};
use crate::diff_options::DiffOptions;
use crate::myers::{self, MatchBlock};

/// Compares two sequences of lines and returns chunks covering both sides
/// completely, in order.
///
/// # Examples
///
/// ```
/// use compare_diff::{ChunkKind, DiffOptions, diff_lines};
///
/// let chunks = diff_lines(&["a", "b", "c"], &["a", "x", "c"], &DiffOptions::default());
/// let kinds: Vec<ChunkKind> = chunks.iter().map(|c| c.kind).collect();
/// assert_eq!(kinds, [ChunkKind::Equal, ChunkKind::Replace, ChunkKind::Equal]);
/// ```
pub fn diff_lines(left: &[&str], right: &[&str], options: &DiffOptions) -> Vec<DiffChunk> {
    let (left_ids, right_ids) = intern_lines(left, right, options);
    let blocks = match_ids(&left_ids, &right_ids);
    chunks_from_blocks(&blocks, left.len(), right.len())
}

/// Maps every line to an integer so that equal lines (under `options`) get
/// equal ids. Diffing integers is far cheaper than diffing strings.
pub(crate) fn intern_lines<'a>(
    left: &[&'a str],
    right: &[&'a str],
    options: &DiffOptions,
) -> (Vec<u32>, Vec<u32>) {
    let mut ids: FxHashMap<Cow<'a, str>, u32> = FxHashMap::default();
    ids.reserve(left.len() + right.len());
    let mut intern = |line: &'a str| -> u32 {
        let key = normalize(line, options);
        let next = ids.len() as u32;
        *ids.entry(key).or_insert(next)
    };
    let left_ids: Vec<u32> = left.iter().map(|line| intern(line)).collect();
    let right_ids: Vec<u32> = right.iter().map(|line| intern(line)).collect();
    (left_ids, right_ids)
}

fn normalize<'a>(line: &'a str, options: &DiffOptions) -> Cow<'a, str> {
    if options.is_exact() {
        return Cow::Borrowed(line);
    }
    let mut normalized: Cow<'a, str> = Cow::Borrowed(line);
    if options.ignore_whitespace && line.contains(char::is_whitespace) {
        normalized = Cow::Owned(line.chars().filter(|c| !c.is_whitespace()).collect());
    }
    if options.ignore_case && normalized.chars().any(char::is_uppercase) {
        normalized = Cow::Owned(normalized.to_lowercase());
    }
    normalized
}

/// Finds matching blocks between two id sequences.
///
/// Lines that occur on only one side can never be part of the longest common
/// subsequence, so they are removed before running Myers. This keeps the edit
/// distance (and therefore the running time) small for files with many unique
/// changed lines.
pub(crate) fn match_ids(left: &[u32], right: &[u32]) -> Vec<MatchBlock> {
    let id_count = left.iter().chain(right).copied().max().map_or(0, |max| max as usize + 1);
    let mut in_left = vec![false; id_count];
    let mut in_right = vec![false; id_count];
    for &id in left {
        in_left[id as usize] = true;
    }
    for &id in right {
        in_right[id as usize] = true;
    }

    let left_kept: Vec<usize> = (0..left.len()).filter(|&i| in_right[left[i] as usize]).collect();
    let right_kept: Vec<usize> = (0..right.len()).filter(|&i| in_left[right[i] as usize]).collect();
    if left_kept.len() == left.len() && right_kept.len() == right.len() {
        return myers::diff(left, right);
    }

    let left_filtered: Vec<u32> = left_kept.iter().map(|&i| left[i]).collect();
    let right_filtered: Vec<u32> = right_kept.iter().map(|&i| right[i]).collect();
    let filtered_blocks = myers::diff(&left_filtered, &right_filtered);

    // Map filtered matches back to original indices, re-coalescing runs that
    // are still contiguous in the original sequences.
    let mut blocks: Vec<MatchBlock> = Vec::with_capacity(filtered_blocks.len());
    for block in &filtered_blocks {
        for offset in 0..block.len {
            let left_start = left_kept[block.left_start + offset];
            let right_start = right_kept[block.right_start + offset];
            if let Some(last) = blocks.last_mut()
                && last.left_start + last.len == left_start
                && last.right_start + last.len == right_start
            {
                last.len += 1;
            } else {
                blocks.push(MatchBlock { left_start, right_start, len: 1 });
            }
        }
    }
    blocks
}

/// Converts matching blocks into chunks covering `0..left_len` and
/// `0..right_len` entirely.
pub(crate) fn chunks_from_blocks(
    blocks: &[MatchBlock],
    left_len: usize,
    right_len: usize,
) -> Vec<DiffChunk> {
    let mut chunks: Vec<DiffChunk> = Vec::with_capacity(blocks.len() * 2 + 1);
    let mut left_pos = 0;
    let mut right_pos = 0;
    let end = MatchBlock { left_start: left_len, right_start: right_len, len: 0 };
    for block in blocks.iter().chain(std::iter::once(&end)) {
        let left = left_pos..block.left_start;
        let right = right_pos..block.right_start;
        let kind = match (left.is_empty(), right.is_empty()) {
            (true, true) => None,
            (false, true) => Some(ChunkKind::Delete),
            (true, false) => Some(ChunkKind::Insert),
            (false, false) => Some(ChunkKind::Replace),
        };
        if let Some(kind) = kind {
            chunks.push(DiffChunk { kind, left, right });
        }
        if block.len > 0 {
            chunks.push(DiffChunk {
                kind: ChunkKind::Equal,
                left: block.left_start..block.left_start + block.len,
                right: block.right_start..block.right_start + block.len,
            });
        }
        left_pos = block.left_start + block.len;
        right_pos = block.right_start + block.len;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(chunks: &[DiffChunk]) -> Vec<ChunkKind> {
        chunks.iter().map(|c| c.kind).collect()
    }

    #[test]
    fn identical() {
        let chunks = diff_lines(&["a", "b"], &["a", "b"], &DiffOptions::default());
        assert_eq!(chunks, vec![DiffChunk { kind: ChunkKind::Equal, left: 0..2, right: 0..2 }]);
    }

    #[test]
    fn both_empty() {
        assert!(diff_lines(&[], &[], &DiffOptions::default()).is_empty());
    }

    #[test]
    fn insert_delete_replace() {
        let left = ["a", "b", "c", "d"];
        let right = ["a", "c", "x", "d", "e"];
        let chunks = diff_lines(&left, &right, &DiffOptions::default());
        assert_eq!(
            chunks,
            vec![
                DiffChunk { kind: ChunkKind::Equal, left: 0..1, right: 0..1 },
                DiffChunk { kind: ChunkKind::Delete, left: 1..2, right: 1..1 },
                DiffChunk { kind: ChunkKind::Equal, left: 2..3, right: 1..2 },
                DiffChunk { kind: ChunkKind::Insert, left: 3..3, right: 2..3 },
                DiffChunk { kind: ChunkKind::Equal, left: 3..4, right: 3..4 },
                DiffChunk { kind: ChunkKind::Insert, left: 4..4, right: 4..5 },
            ]
        );
    }

    #[test]
    fn unique_lines_are_discarded_without_losing_matches() {
        let left = ["u1", "a", "u2", "b", "u3"];
        let right = ["v1", "a", "b", "v2"];
        let chunks = diff_lines(&left, &right, &DiffOptions::default());
        assert_eq!(
            kinds(&chunks),
            [
                ChunkKind::Replace,
                ChunkKind::Equal,
                ChunkKind::Delete,
                ChunkKind::Equal,
                ChunkKind::Replace,
            ]
        );
    }

    #[test]
    fn ignore_whitespace_and_case() {
        let left = ["Hello  World", "x"];
        let right = ["hello world", "x"];
        let exact = diff_lines(&left, &right, &DiffOptions::default());
        assert_eq!(kinds(&exact), [ChunkKind::Replace, ChunkKind::Equal]);

        let options = DiffOptions { ignore_whitespace: true, ignore_case: true };
        let loose = diff_lines(&left, &right, &options);
        assert_eq!(kinds(&loose), [ChunkKind::Equal]);
    }
}
