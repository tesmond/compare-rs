//! Linear-space Myers diff.
//!
//! Implements the divide-and-conquer "middle snake" variant from Eugene Myers,
//! "An O(ND) Difference Algorithm and Its Variations" (1986). Time is
//! O((N + M) * D) and memory is O(N + M), where D is the edit distance.
//!
//! To keep pathological inputs (huge files that are almost entirely different)
//! responsive, the search for a middle snake gives up after a cost limit and
//! splits at the furthest-reaching point instead. The result is then still a
//! valid diff, just not guaranteed to be minimal.

use std::ops::Range;

/// A run of `len` elements that are equal on both sides, starting at
/// `left[left_start]` and `right[right_start]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchBlock {
    pub left_start: usize,
    pub right_start: usize,
    pub len: usize,
}

/// Smallest middle-snake search depth before the cost limit can kick in.
const MIN_COST_LIMIT: isize = 1024;

/// Computes the matching blocks between `left` and `right`.
///
/// Blocks are returned in increasing order on both sides, never overlap, and
/// adjacent blocks are coalesced.
pub fn diff<T: PartialEq>(left: &[T], right: &[T]) -> Vec<MatchBlock> {
    let max_d = max_d(left.len(), right.len());
    let total = (left.len() + right.len()) as isize;
    let mut myers = Myers {
        left,
        right,
        forward: DiagonalVec::new(max_d),
        backward: DiagonalVec::new(max_d),
        cost_limit: MIN_COST_LIMIT.max(total.isqrt() * 4),
        blocks: Vec::new(),
    };
    myers.conquer(0..left.len(), 0..right.len());
    coalesce(myers.blocks)
}

/// Upper bound on the number of `d` iterations needed for inputs of these sizes.
fn max_d(left_len: usize, right_len: usize) -> usize {
    (left_len + right_len).div_ceil(2) + 1
}

/// Furthest-reaching x coordinate on each diagonal `k`, indexed from `-max_d`
/// to `max_d`.
struct DiagonalVec {
    offset: isize,
    values: Vec<isize>,
}

impl DiagonalVec {
    fn new(max_d: usize) -> Self {
        Self { offset: max_d as isize, values: vec![0; 2 * max_d + 2] }
    }

    fn get(&self, k: isize) -> isize {
        self.values[(k + self.offset) as usize]
    }

    fn set(&mut self, k: isize, x: isize) {
        self.values[(k + self.offset) as usize] = x;
    }
}

struct Myers<'a, T> {
    left: &'a [T],
    right: &'a [T],
    forward: DiagonalVec,
    backward: DiagonalVec,
    cost_limit: isize,
    blocks: Vec<MatchBlock>,
}

impl<T: PartialEq> Myers<'_, T> {
    fn conquer(&mut self, mut left: Range<usize>, mut right: Range<usize>) {
        let prefix = common_prefix_len(&self.left[left.clone()], &self.right[right.clone()]);
        if prefix > 0 {
            self.blocks.push(MatchBlock {
                left_start: left.start,
                right_start: right.start,
                len: prefix,
            });
            left.start += prefix;
            right.start += prefix;
        }

        let suffix = common_suffix_len(&self.left[left.clone()], &self.right[right.clone()]);
        left.end -= suffix;
        right.end -= suffix;

        // If no split point is found the remaining range is treated as replaced.
        if !left.is_empty()
            && !right.is_empty()
            && let Some((x, y)) = self.find_middle_snake(left.clone(), right.clone())
        {
            self.conquer(left.start..x, right.start..y);
            self.conquer(x..left.end, y..right.end);
        }

        if suffix > 0 {
            self.blocks.push(MatchBlock {
                left_start: left.end,
                right_start: right.end,
                len: suffix,
            });
        }
    }

    /// Finds a point `(x, y)` (absolute indices) that lies on an optimal edit
    /// path and splits the problem into two strictly smaller ones.
    ///
    /// Both ranges must be non-empty and must not share a common prefix or
    /// suffix.
    fn find_middle_snake(
        &mut self,
        left: Range<usize>,
        right: Range<usize>,
    ) -> Option<(usize, usize)> {
        let n = left.len() as isize;
        let m = right.len() as isize;
        let delta = n - m;
        let odd = delta & 1 != 0;
        let d_max = max_d(left.len(), right.len()) as isize;

        self.forward.set(1, 0);
        self.backward.set(1, 0);

        for d in 0..d_max {
            if d > self.cost_limit {
                return self
                    .furthest_forward_point(d - 1, n, m)
                    .map(|(x, y)| ((x as usize) + left.start, (y as usize) + right.start));
            }

            // Forward search from the top-left corner.
            let mut k = d;
            while k >= -d {
                let mut x =
                    if k == -d || (k != d && self.forward.get(k - 1) < self.forward.get(k + 1)) {
                        self.forward.get(k + 1)
                    } else {
                        self.forward.get(k - 1) + 1
                    };
                let y = x - k;
                let (snake_x, snake_y) = (x, y);
                if x < n && y < m {
                    x += common_prefix_len(
                        &self.left[left.start + x as usize..left.end],
                        &self.right[right.start + y as usize..right.end],
                    ) as isize;
                }
                self.forward.set(k, x);
                if odd
                    && (k - delta).abs() < d
                    && self.forward.get(k) + self.backward.get(delta - k) >= n
                {
                    return Some((left.start + snake_x as usize, right.start + snake_y as usize));
                }
                k -= 2;
            }

            // Backward search from the bottom-right corner, in reversed
            // coordinates.
            let mut k = d;
            while k >= -d {
                let mut x =
                    if k == -d || (k != d && self.backward.get(k - 1) < self.backward.get(k + 1)) {
                        self.backward.get(k + 1)
                    } else {
                        self.backward.get(k - 1) + 1
                    };
                let mut y = x - k;
                if x < n && y < m {
                    let advance = common_suffix_len(
                        &self.left[left.start..left.end - x as usize],
                        &self.right[right.start..right.end - y as usize],
                    ) as isize;
                    x += advance;
                    y += advance;
                }
                self.backward.set(k, x);
                if !odd
                    && (k - delta).abs() <= d
                    && self.backward.get(k) + self.forward.get(delta - k) >= n
                {
                    return Some((left.start + (n - x) as usize, right.start + (m - y) as usize));
                }
                k -= 2;
            }
        }
        None
    }

    /// Returns the in-bounds forward point that has made the most progress
    /// after `d` iterations, if it splits the problem.
    fn furthest_forward_point(&self, d: isize, n: isize, m: isize) -> Option<(isize, isize)> {
        let mut best: Option<(isize, isize)> = None;
        let mut k = d;
        while k >= -d {
            let x = self.forward.get(k).min(n);
            let y = x - k;
            if (0..=m).contains(&y) && best.is_none_or(|(bx, by)| x + y > bx + by) {
                best = Some((x, y));
            }
            k -= 2;
        }
        best.filter(|&(x, y)| (x, y) != (0, 0) && (x, y) != (n, m))
    }
}

fn common_prefix_len<T: PartialEq>(left: &[T], right: &[T]) -> usize {
    left.iter().zip(right).take_while(|(a, b)| a == b).count()
}

fn common_suffix_len<T: PartialEq>(left: &[T], right: &[T]) -> usize {
    left.iter().rev().zip(right.iter().rev()).take_while(|(a, b)| a == b).count()
}

fn coalesce(blocks: Vec<MatchBlock>) -> Vec<MatchBlock> {
    let mut result: Vec<MatchBlock> = Vec::with_capacity(blocks.len());
    for block in blocks {
        if block.len == 0 {
            continue;
        }
        if let Some(last) = result.last_mut()
            && last.left_start + last.len == block.left_start
            && last.right_start + last.len == block.right_start
        {
            last.len += block.len;
            continue;
        }
        result.push(block);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcs_len(a: &[u8], b: &[u8]) -> usize {
        let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for i in 1..=a.len() {
            for j in 1..=b.len() {
                dp[i][j] = if a[i - 1] == b[j - 1] {
                    dp[i - 1][j - 1] + 1
                } else {
                    dp[i - 1][j].max(dp[i][j - 1])
                };
            }
        }
        dp[a.len()][b.len()]
    }

    fn assert_valid(a: &[u8], b: &[u8], blocks: &[MatchBlock]) {
        let mut left_end = 0;
        let mut right_end = 0;
        for block in blocks {
            assert!(block.left_start >= left_end, "blocks overlap on the left: {blocks:?}");
            assert!(block.right_start >= right_end, "blocks overlap on the right: {blocks:?}");
            for i in 0..block.len {
                assert_eq!(a[block.left_start + i], b[block.right_start + i]);
            }
            left_end = block.left_start + block.len;
            right_end = block.right_start + block.len;
        }
    }

    /// Small deterministic xorshift generator so tests need no dependencies.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn sequence(&mut self, max_len: u64, alphabet: u64) -> Vec<u8> {
            let len = self.next() % max_len;
            (0..len).map(|_| (self.next() % alphabet) as u8).collect()
        }
    }

    #[test]
    fn empty_inputs() {
        assert!(diff::<u8>(&[], &[]).is_empty());
        assert!(diff(b"abc", b"").is_empty());
        assert!(diff(b"", b"abc").is_empty());
    }

    #[test]
    fn identical_inputs_form_one_block() {
        let blocks = diff(b"abcdef", b"abcdef");
        assert_eq!(blocks, vec![MatchBlock { left_start: 0, right_start: 0, len: 6 }]);
    }

    #[test]
    fn classic_example_is_minimal() {
        let a = b"abcabba";
        let b = b"cbabac";
        let blocks = diff(a, b);
        assert_valid(a, b, &blocks);
        assert_eq!(blocks.iter().map(|b| b.len).sum::<usize>(), 4);
    }

    #[test]
    fn random_inputs_match_brute_force_lcs() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..2000 {
            let a = rng.sequence(40, 4);
            let b = rng.sequence(40, 4);
            let blocks = diff(&a, &b);
            assert_valid(&a, &b, &blocks);
            let matched: usize = blocks.iter().map(|b| b.len).sum();
            assert_eq!(matched, lcs_len(&a, &b), "a={a:?} b={b:?}");
        }
    }

    #[test]
    fn cost_limit_still_yields_valid_diff() {
        let mut rng = Rng(42);
        let a = rng.sequence(20_000, 50);
        let b = rng.sequence(20_000, 50);
        let blocks = diff(&a, &b);
        assert_valid(&a, &b, &blocks);
    }
}
