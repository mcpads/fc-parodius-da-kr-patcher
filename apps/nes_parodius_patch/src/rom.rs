use std::{fs, path::Path};

use anyhow::{Context, Result, bail, ensure};

use crate::sha1_hex;

pub const HEADER_SIZE: usize = 16;
pub const PRG_UNIT: usize = 16 * 1024;
pub const CHR_UNIT: usize = 8 * 1024;
pub const PRG_BANK_SIZE: usize = 8 * 1024;
pub const SOURCE_PRG_SIZE: usize = 128 * 1024;
pub const EXPANDED_PRG_SIZE: usize = 256 * 1024;
pub const SOURCE_CHR_SIZE: usize = 128 * 1024;
pub const EXPANDED_CHR_SIZE: usize = 256 * 1024;

pub const EXPECTED_SOURCE_SHA1: &str = "fc8ee2c4d869b1d09ad4d66d3c2d557336a6560d";
pub const EXPECTED_HEADER: [u8; HEADER_SIZE] = [
    0x4E, 0x45, 0x53, 0x1A, 0x08, 0x10, 0x70, 0x18, 0x20, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x01,
];

#[derive(Debug, Clone)]
pub struct Rom {
    data: Vec<u8>,
    header: [u8; HEADER_SIZE],
    prg_range: std::ops::Range<usize>,
    chr_range: std::ops::Range<usize>,
    mapper: u16,
    submapper: u8,
}

impl Rom {
    pub fn from_path(path: &Path) -> Result<Self> {
        let data = fs::read(path).with_context(|| format!("read ROM {}", path.display()))?;
        Self::parse(data).with_context(|| format!("parse ROM {}", path.display()))
    }

    pub fn parse(data: Vec<u8>) -> Result<Self> {
        ensure!(
            data.len() >= HEADER_SIZE && &data[..4] == b"NES\x1A",
            "not an iNES/NES 2.0 image"
        );
        let header: [u8; HEADER_SIZE] = data[..HEADER_SIZE].try_into().unwrap();
        ensure!(
            header[7] & 0x0C == 0x08,
            "only NES 2.0 images are supported"
        );
        ensure!(
            header[6] & 0x04 == 0,
            "trainer-bearing images are unsupported"
        );
        let prg_size = normal_size(header[4], header[9] & 0x0F, PRG_UNIT, "PRG")?;
        let chr_size = normal_size(header[5], header[9] >> 4, CHR_UNIT, "CHR")?;
        let payload_end = HEADER_SIZE + prg_size + chr_size;
        ensure!(
            data.len() == payload_end,
            "header requires {payload_end} bytes, found {}",
            data.len()
        );
        let mapper = ((header[6] >> 4) as u16)
            | ((header[7] & 0xF0) as u16)
            | (((header[8] & 0x0F) as u16) << 8);
        let submapper = header[8] >> 4;
        Ok(Self {
            data,
            header,
            prg_range: HEADER_SIZE..HEADER_SIZE + prg_size,
            chr_range: HEADER_SIZE + prg_size..payload_end,
            mapper,
            submapper,
        })
    }

    pub fn verify_supported_japanese(&self) -> Result<()> {
        let actual_sha1 = sha1_hex(&self.data);
        ensure!(
            actual_sha1 == EXPECTED_SOURCE_SHA1,
            "source SHA-1 mismatch: expected {EXPECTED_SOURCE_SHA1}, found {actual_sha1}"
        );
        ensure!(
            self.header == EXPECTED_HEADER,
            "source header mismatch: expected {}, found {}",
            hex(&EXPECTED_HEADER),
            hex(&self.header)
        );
        ensure!(
            (self.mapper, self.submapper) == (23, 2),
            "mapper mismatch: expected 23.2, found {}.{}",
            self.mapper,
            self.submapper
        );
        ensure!(
            self.prg().len() == SOURCE_PRG_SIZE,
            "unexpected source PRG size"
        );
        ensure!(
            self.chr().len() == SOURCE_CHR_SIZE,
            "unexpected source CHR size"
        );
        Ok(())
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn payload(&self) -> &[u8] {
        &self.data[HEADER_SIZE..]
    }

    pub fn header(&self) -> &[u8; HEADER_SIZE] {
        &self.header
    }

    pub fn prg(&self) -> &[u8] {
        &self.data[self.prg_range.clone()]
    }

    pub fn chr(&self) -> &[u8] {
        &self.data[self.chr_range.clone()]
    }

    pub fn mapper(&self) -> u16 {
        self.mapper
    }

    pub fn submapper(&self) -> u8 {
        self.submapper
    }

    pub fn format_name(&self) -> &'static str {
        "NES 2.0"
    }
}

pub fn expand_prg(prg: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        prg.len() == SOURCE_PRG_SIZE,
        "source PRG must be {SOURCE_PRG_SIZE} bytes, found {}",
        prg.len()
    );
    let movable_end = 14 * PRG_BANK_SIZE;
    let mut expanded = Vec::with_capacity(EXPANDED_PRG_SIZE);
    expanded.extend_from_slice(&prg[..movable_end]);
    expanded.resize(30 * PRG_BANK_SIZE, 0);
    expanded.extend_from_slice(&prg[movable_end..]);
    ensure!(
        expanded.len() == EXPANDED_PRG_SIZE,
        "expanded PRG size invariant failed"
    );
    Ok(expanded)
}

fn normal_size(lsb: u8, msb_nibble: u8, unit: usize, label: &str) -> Result<usize> {
    if msb_nibble == 0x0F {
        bail!("{label} uses unsupported NES 2.0 exponent notation");
    }
    Ok(((lsb as usize) | ((msb_nibble as usize) << 8)) * unit)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_fixed_banks_and_zero_fills_gap() {
        let source: Vec<u8> = (0_u8..16)
            .flat_map(|bank| std::iter::repeat_n(bank, PRG_BANK_SIZE))
            .collect();
        let result = expand_prg(&source).unwrap();

        assert_eq!(result.len(), EXPANDED_PRG_SIZE);
        assert_eq!(&result[..14 * PRG_BANK_SIZE], &source[..14 * PRG_BANK_SIZE]);
        assert!(
            result[14 * PRG_BANK_SIZE..30 * PRG_BANK_SIZE]
                .iter()
                .all(|byte| *byte == 0)
        );
        assert_eq!(&result[30 * PRG_BANK_SIZE..], &source[14 * PRG_BANK_SIZE..]);
    }

    #[test]
    fn rejects_wrong_prg_size() {
        assert!(expand_prg(&vec![0; SOURCE_PRG_SIZE - 1]).is_err());
    }
}
