//! Byte-wise comparison of binary files.

/// Number of bytes shown on each row of a hex view.
pub const BYTES_PER_ROW: usize = 16;

/// Two byte buffers compared position by position.
#[derive(Clone, Debug, Default)]
pub struct BinaryDiff {
    pub left: Vec<u8>,
    pub right: Vec<u8>,
    /// Number of offsets whose bytes differ, counting bytes present on only
    /// one side.
    pub differing_bytes: u64,
    /// Hex rows (of [`BYTES_PER_ROW`] bytes) that contain a difference, in order.
    pub differing_rows: Vec<usize>,
}

impl BinaryDiff {
    pub fn new(left: Vec<u8>, right: Vec<u8>) -> Self {
        let common = left.len().min(right.len());
        let longest = left.len().max(right.len());
        let mut differing_bytes = (longest - common) as u64;
        let mut differing_rows: Vec<usize> = Vec::new();

        let left_rows = left[..common].chunks(BYTES_PER_ROW);
        let right_rows = right[..common].chunks(BYTES_PER_ROW);
        for (row, (l, r)) in left_rows.zip(right_rows).enumerate() {
            if l != r {
                differing_bytes += l.iter().zip(r).filter(|(a, b)| a != b).count() as u64;
                differing_rows.push(row);
            }
        }
        // Every row from the one holding the first unmatched byte onwards differs.
        if longest > common {
            let mut tail_start = common / BYTES_PER_ROW;
            if differing_rows.last() == Some(&tail_start) {
                tail_start += 1;
            }
            differing_rows.extend(tail_start..longest.div_ceil(BYTES_PER_ROW));
        }

        Self { left, right, differing_bytes, differing_rows }
    }

    /// Index into `differing_rows` of the start of the run of adjacent
    /// differing rows after the one holding `current`, wrapping to the
    /// first run. `None` if nothing differs.
    pub fn next_run(&self, current: Option<usize>) -> Option<usize> {
        let rows = &self.differing_rows;
        if rows.is_empty() {
            return None;
        }
        let Some(mut end) = current.filter(|&current| current < rows.len()) else {
            return Some(0);
        };
        while end + 1 < rows.len() && rows[end + 1] == rows[end] + 1 {
            end += 1;
        }
        Some((end + 1) % rows.len())
    }

    /// Like [`BinaryDiff::next_run`], for the run before the one holding
    /// `current`, wrapping to the last run.
    pub fn prev_run(&self, current: Option<usize>) -> Option<usize> {
        let rows = &self.differing_rows;
        let last = rows.len().checked_sub(1)?;
        let run_start = |mut index: usize| {
            while index > 0 && rows[index - 1] + 1 == rows[index] {
                index -= 1;
            }
            index
        };
        let before = match current.filter(|&current| current <= last).map(run_start) {
            None | Some(0) => last,
            Some(start) => start - 1,
        };
        Some(run_start(before))
    }

    /// Number of hex rows needed to show the longer side.
    pub fn row_count(&self) -> usize {
        self.left.len().max(self.right.len()).div_ceil(BYTES_PER_ROW)
    }

    /// Percentage of offsets that differ, relative to the longer side.
    pub fn difference_percent(&self) -> f64 {
        let longest = self.left.len().max(self.right.len());
        if longest == 0 { 0.0 } else { self.differing_bytes as f64 * 100.0 / longest as f64 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_differences() {
        let left = vec![0u8; 40];
        let mut right = left.clone();
        right[3] = 1;
        right[20] = 1;
        let diff = BinaryDiff::new(left, right);
        assert_eq!(diff.differing_bytes, 2);
        assert_eq!(diff.differing_rows, [0, 1]);
        assert_eq!(diff.row_count(), 3);
    }

    #[test]
    fn steps_between_runs_of_differing_rows() {
        let diff = BinaryDiff { differing_rows: vec![2, 3, 4, 9, 20, 21], ..Default::default() };
        assert_eq!(diff.next_run(None), Some(0));
        assert_eq!(diff.next_run(Some(0)), Some(3));
        assert_eq!(diff.next_run(Some(1)), Some(3));
        assert_eq!(diff.next_run(Some(3)), Some(4));
        assert_eq!(diff.next_run(Some(4)), Some(0));
        assert_eq!(diff.prev_run(None), Some(4));
        assert_eq!(diff.prev_run(Some(5)), Some(3));
        assert_eq!(diff.prev_run(Some(3)), Some(0));
        assert_eq!(diff.prev_run(Some(1)), Some(4));
        assert_eq!(BinaryDiff::default().next_run(None), None);
        assert_eq!(BinaryDiff::default().prev_run(None), None);
    }

    #[test]
    fn tail_rows_differ() {
        let left = vec![7u8; 20];
        let mut right = left.clone();
        right[17] = 0;
        right.extend([1u8; 30]);
        let diff = BinaryDiff::new(left, right);
        assert_eq!(diff.differing_bytes, 31);
        assert_eq!(diff.differing_rows, [1, 2, 3]);
    }
}
