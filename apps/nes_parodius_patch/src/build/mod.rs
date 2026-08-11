use anyhow::{Result, ensure};

use crate::{
    bps::SourceMapping,
    rom::{
        EXPANDED_CHR_SIZE, EXPANDED_PRG_SIZE, HEADER_SIZE, PRG_BANK_SIZE, Rom, SOURCE_CHR_SIZE,
        SOURCE_PRG_SIZE, expand_prg,
    },
};

pub mod ending_draft_poc;
pub mod ending_poc;
pub mod korean_draft;
pub mod reference_labels;
pub mod roulette_label;
pub mod roulette_render;
pub mod roulette_sweep;
pub mod title_poc;

pub fn headerless_bps_mappings() -> [SourceMapping; 3] {
    const MOVABLE_PRG_LEN: usize = 14 * PRG_BANK_SIZE;
    const FIXED_PRG_LEN: usize = 2 * PRG_BANK_SIZE;
    [
        SourceMapping::new(0, HEADER_SIZE, MOVABLE_PRG_LEN),
        SourceMapping::new(
            MOVABLE_PRG_LEN,
            HEADER_SIZE + 30 * PRG_BANK_SIZE,
            FIXED_PRG_LEN,
        ),
        SourceMapping::new(
            SOURCE_PRG_SIZE,
            HEADER_SIZE + EXPANDED_PRG_SIZE,
            SOURCE_CHR_SIZE,
        ),
    ]
}

pub fn build_expanded(source: &Rom) -> Result<Vec<u8>> {
    source.verify_supported_japanese()?;
    let mut header = *source.header();
    header[4] = (EXPANDED_PRG_SIZE / crate::rom::PRG_UNIT) as u8;

    let mut output =
        Vec::with_capacity(crate::rom::HEADER_SIZE + EXPANDED_PRG_SIZE + source.chr().len());
    output.extend_from_slice(&header);
    output.extend_from_slice(&expand_prg(source.prg())?);
    output.extend_from_slice(source.chr());
    ensure!(
        output.len() == crate::rom::HEADER_SIZE + EXPANDED_PRG_SIZE + source.chr().len(),
        "expanded image size invariant failed"
    );
    Ok(output)
}

pub fn build_expanded_chr(source: &Rom) -> Result<Vec<u8>> {
    source.verify_supported_japanese()?;
    let mut header = *source.header();
    header[4] = (EXPANDED_PRG_SIZE / crate::rom::PRG_UNIT) as u8;
    header[5] = (EXPANDED_CHR_SIZE / crate::rom::CHR_UNIT) as u8;

    let mut output =
        Vec::with_capacity(crate::rom::HEADER_SIZE + EXPANDED_PRG_SIZE + EXPANDED_CHR_SIZE);
    output.extend_from_slice(&header);
    output.extend_from_slice(&expand_prg(source.prg())?);
    output.extend_from_slice(source.chr());
    output.resize(
        crate::rom::HEADER_SIZE + EXPANDED_PRG_SIZE + EXPANDED_CHR_SIZE,
        0,
    );
    ensure!(
        output.len() == crate::rom::HEADER_SIZE + EXPANDED_PRG_SIZE + EXPANDED_CHR_SIZE,
        "expanded CHR image size invariant failed"
    );
    Ok(output)
}
