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
