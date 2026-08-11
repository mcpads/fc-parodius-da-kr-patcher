use std::ops::Range;

use anyhow::{Result, ensure};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteReport {
    pub label: String,
    pub offset: usize,
    pub len: usize,
}

#[derive(Debug, Clone)]
pub struct TrackedImage {
    data: Vec<u8>,
    reports: Vec<WriteReport>,
    ranges: Vec<Range<usize>>,
}

impl TrackedImage {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            reports: Vec::new(),
            ranges: Vec::new(),
        }
    }

    pub fn write_expect(
        &mut self,
        label: impl Into<String>,
        offset: usize,
        expected: &[u8],
        replacement: &[u8],
    ) -> Result<()> {
        ensure!(
            expected.len() == replacement.len(),
            "Expected Write length mismatch"
        );
        let end = offset
            .checked_add(expected.len())
            .ok_or_else(|| anyhow::anyhow!("Expected Write range overflow"))?;
        ensure!(
            end <= self.data.len(),
            "Expected Write is outside the image"
        );
        ensure!(
            self.data[offset..end] == *expected,
            "Expected Write precondition failed at {offset:#X}: expected {}, found {}",
            hex(expected),
            hex(&self.data[offset..end])
        );
        ensure!(
            self.ranges
                .iter()
                .all(|range| end <= range.start || offset >= range.end),
            "Expected Write overlaps a previous tracked write"
        );
        self.data[offset..end].copy_from_slice(replacement);
        self.reports.push(WriteReport {
            label: label.into(),
            offset,
            len: expected.len(),
        });
        self.ranges.push(offset..end);
        Ok(())
    }

    pub fn check_untracked_writes(&self, baseline: &[u8]) -> Result<()> {
        ensure!(self.data.len() == baseline.len(), "baseline size mismatch");
        for (offset, (before, after)) in baseline.iter().zip(&self.data).enumerate() {
            if before != after {
                ensure!(
                    self.ranges.iter().any(|range| range.contains(&offset)),
                    "untracked write at {offset:#X}: {before:02X} -> {after:02X}"
                );
            }
        }
        Ok(())
    }

    pub fn reports(&self) -> &[WriteReport] {
        &self.reports
    }

    pub fn into_data(self) -> Vec<u8> {
        self.data
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_write_and_audit_accept_only_tracked_delta() {
        let baseline = vec![0_u8; 8];
        let mut image = TrackedImage::new(baseline.clone());
        image.write_expect("test", 2, &[0, 0], &[1, 2]).unwrap();
        image.check_untracked_writes(&baseline).unwrap();
        assert_eq!(image.reports()[0].label, "test");
    }

    #[test]
    fn rejects_bad_precondition_and_overlap() {
        let mut image = TrackedImage::new(vec![0_u8; 8]);
        assert!(image.write_expect("bad", 0, &[1], &[2]).is_err());
        image.write_expect("first", 1, &[0, 0], &[1, 1]).unwrap();
        assert!(image.write_expect("overlap", 2, &[1], &[2]).is_err());
    }
}
