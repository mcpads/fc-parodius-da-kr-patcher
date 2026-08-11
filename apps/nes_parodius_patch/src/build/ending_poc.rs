use std::collections::BTreeMap;

use anyhow::{Result, ensure};

use crate::{
    ending::{
        ENDING_BANK, ENDING_REGION_END, POINTER_TABLE_CPU, SONG_POINTER_ENTRIES, SONG_REGION_START,
        cpu_offset, source_song_assets,
    },
    font::PocSpec,
    rom::{EXPANDED_PRG_SIZE, HEADER_SIZE, PRG_BANK_SIZE, Rom},
    tracked::{TrackedImage, WriteReport},
};

use super::build_expanded;

const ENDING_REGION_SIZE: usize = (ENDING_REGION_END - SONG_REGION_START) as usize;
const CHR_PHYSICAL_BANK: usize = 0x2C;
const CHR_BANK_SIZE: usize = 1024;
const TILE_SIZE: usize = 16;

#[derive(Debug, Clone)]
pub struct EndingPocReport {
    pub text: String,
    pub glyph_count: usize,
    pub blanked_records: usize,
    pub used: usize,
    pub remaining: usize,
    pub target_old_pointer: u16,
    pub target_new_pointer: u16,
}

#[derive(Debug, Clone)]
pub struct EndingPocBuild {
    pub data: Vec<u8>,
    pub report: EndingPocReport,
    pub writes: Vec<WriteReport>,
}

pub fn build(source: &Rom, spec: &PocSpec) -> Result<EndingPocBuild> {
    source.verify_supported_japanese()?;
    let glyphs = spec.validate()?;
    let baseline = build_expanded(source)?;
    let bank_start = ENDING_BANK * PRG_BANK_SIZE;
    let source_bank = &source.prg()[bank_start..bank_start + PRG_BANK_SIZE];
    let (patched_bank, mut report) = compile_ending_bank(source_bank, spec)?;
    report.glyph_count = glyphs.tiles.len();

    let mut image = TrackedImage::new(baseline.clone());
    let output_bank_start = HEADER_SIZE + bank_start;
    let region = cpu_offset(SONG_REGION_START)?;
    image.write_expect(
        "repack Japanese ending assets",
        output_bank_start + region,
        &source_bank[region..region + ENDING_REGION_SIZE],
        &patched_bank[region..region + ENDING_REGION_SIZE],
    )?;
    let table = cpu_offset(POINTER_TABLE_CPU)?;
    for (index, _) in SONG_POINTER_ENTRIES {
        let offset = table + index as usize;
        image.write_expect(
            format!("relocate ending pointer {index:02X}"),
            output_bank_start + offset,
            &source_bank[offset..offset + 2],
            &patched_bank[offset..offset + 2],
        )?;
    }

    let chr_start = HEADER_SIZE + EXPANDED_PRG_SIZE;
    for (code, tile) in glyphs.tiles {
        let source_chr_offset = CHR_PHYSICAL_BANK * CHR_BANK_SIZE + code as usize * TILE_SIZE;
        image.write_expect(
            format!("install Korean glyph {code:02X}"),
            chr_start + source_chr_offset,
            &source.chr()[source_chr_offset..source_chr_offset + TILE_SIZE],
            &tile,
        )?;
    }
    image.check_untracked_writes(&baseline)?;
    let writes = image.reports().to_vec();
    Ok(EndingPocBuild {
        data: image.into_data(),
        report,
        writes,
    })
}

fn compile_ending_bank(bank: &[u8], spec: &PocSpec) -> Result<(Vec<u8>, EndingPocReport)> {
    ensure!(bank.len() == PRG_BANK_SIZE, "P08 must be exactly 8 KiB");
    spec.validate()?;
    ensure!(
        SONG_POINTER_ENTRIES
            .iter()
            .any(|(index, _)| *index == spec.target_normalized_index),
        "target_normalized_index is outside the proven ending set"
    );
    let assets = source_song_assets(bank)?;

    let target_pointer = SONG_POINTER_ENTRIES
        .iter()
        .find(|(index, _)| *index == spec.target_normalized_index)
        .map(|(_, pointer)| *pointer)
        .unwrap();
    let target_asset = &assets[&target_pointer];
    for record_number in &spec.blank_records {
        ensure!(
            *record_number < target_asset.records.len(),
            "blank record {record_number} is outside the selected ending asset"
        );
    }
    let mut block = Vec::new();
    let mut relocated = BTreeMap::new();
    let mut replaced = false;
    let mut blanked_records = 0;
    for asset in assets.values() {
        relocated.insert(asset.pointer, SONG_REGION_START + block.len() as u16);
        ensure!(
            asset.pointer != target_pointer || spec.target_record < asset.records.len(),
            "target_record is outside the selected ending asset"
        );
        for (record_number, record) in asset.records.iter().enumerate() {
            block.extend_from_slice(&[record.coord0, record.coord1]);
            if asset.pointer == target_pointer && record_number == spec.target_record {
                block.extend_from_slice(&spec.tokens);
                replaced = true;
            } else if asset.pointer == target_pointer && spec.blank_records.contains(&record_number)
            {
                block.extend(std::iter::repeat_n(0, record.tokens.len()));
                blanked_records += 1;
            } else {
                block.extend_from_slice(&record.tokens);
            }
            block.push(record.terminator);
        }
    }
    ensure!(replaced, "target ending record was not found");
    ensure!(
        block.len() <= ENDING_REGION_SIZE,
        "compiled ending uses {} bytes; proven envelope is {ENDING_REGION_SIZE}",
        block.len()
    );

    let mut output = bank.to_vec();
    let region = cpu_offset(SONG_REGION_START)?;
    output[region..region + block.len()].copy_from_slice(&block);
    output[region + block.len()..region + ENDING_REGION_SIZE].fill(0xFF);
    let table = cpu_offset(POINTER_TABLE_CPU)?;
    for (index, old_pointer) in SONG_POINTER_ENTRIES {
        let offset = table + index as usize;
        output[offset..offset + 2].copy_from_slice(&relocated[&old_pointer].to_le_bytes());
    }
    Ok((
        output,
        EndingPocReport {
            text: spec.text.clone(),
            glyph_count: 0,
            blanked_records,
            used: block.len(),
            remaining: ENDING_REGION_SIZE - block.len(),
            target_old_pointer: target_pointer,
            target_new_pointer: relocated[&target_pointer],
        },
    ))
}

#[cfg(test)]
mod tests {
    use crate::font::GlyphSpec;

    use super::*;

    fn spec(tokens: Vec<u8>) -> PocSpec {
        PocSpec {
            format_version: 2,
            target_normalized_index: 0x42,
            target_record: 2,
            blank_records: vec![0, 1],
            text: "파".repeat(tokens.len()),
            tokens,
            glyphs: vec![GlyphSpec {
                code: 1,
                character: '파',
            }],
        }
    }

    fn synthetic_bank() -> Vec<u8> {
        let mut bank = vec![0_u8; PRG_BANK_SIZE];
        let unique = [
            0x8355, 0x8368, 0x8388, 0x839D, 0x83C8, 0x83DB, 0x83EA, 0x8410, 0x842D, 0x8443,
        ];
        for pair in unique.windows(2) {
            let pointer = pair[0];
            let size = (pair[1] - pair[0]) as usize;
            let mut bytes = Vec::new();
            if pointer == 0x8368 {
                bytes.extend_from_slice(&[0xC4, 0x22, 0x3D, 0xFF]);
                bytes.extend_from_slice(&[0xD1, 0x22, 0x3E, 0xFF]);
                bytes.extend_from_slice(&[0xE4, 0x22]);
                bytes.resize(size - 1, 0x19);
                bytes.push(0xFE);
            } else {
                bytes.extend_from_slice(&[0xE4, 0x22]);
                bytes.resize(size - 1, 0x19);
                bytes.push(0xFE);
            }
            let start = cpu_offset(pointer).unwrap();
            bank[start..start + size].copy_from_slice(&bytes);
        }
        let table = cpu_offset(POINTER_TABLE_CPU).unwrap();
        for (index, pointer) in SONG_POINTER_ENTRIES {
            let offset = table + index as usize;
            bank[offset..offset + 2].copy_from_slice(&pointer.to_le_bytes());
        }
        bank
    }

    #[test]
    fn replaces_target_and_relocates_proven_pointer_set() {
        let (bank, report) = compile_ending_bank(&synthetic_bank(), &spec(vec![1, 1])).unwrap();
        let target =
            crate::ending::parse_postfixed_asset(&bank, report.target_new_pointer).unwrap();
        assert!(target.records[0].tokens.iter().all(|token| *token == 0));
        assert!(target.records[1].tokens.iter().all(|token| *token == 0));
        assert_eq!(target.records[2].tokens, vec![1, 1]);
        assert_eq!(report.blanked_records, 2);
        assert_eq!(report.target_old_pointer, 0x8368);
        assert_eq!(report.target_new_pointer, 0x8368);
        assert!(report.remaining > 0);
    }

    #[test]
    fn rejects_ending_envelope_overflow() {
        let error =
            compile_ending_bank(&synthetic_bank(), &spec(vec![1; ENDING_REGION_SIZE])).unwrap_err();
        assert!(error.to_string().contains("proven envelope"));
    }
}
