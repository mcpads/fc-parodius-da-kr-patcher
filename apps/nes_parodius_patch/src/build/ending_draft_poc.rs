use std::collections::BTreeMap;

use anyhow::{Result, ensure};

use crate::{
    ending::{
        ENDING_BANK, ENDING_REGION_END, INTRO_STREAM_START, POINTER_TABLE_CPU,
        SEQUENCE_POINTER_ENTRIES, SEQUENCE_REGION_START, cpu_offset, parse_prefixed_asset,
    },
    font::rasterize_dalmoori_text,
    rom::{EXPANDED_PRG_SIZE, HEADER_SIZE, PRG_BANK_SIZE, Rom},
    tracked::{TrackedImage, WriteReport},
    translation::{EndingTranslation, TranslationPlan, plan},
};

use super::build_expanded_chr;

const INTRO_REGION_SIZE: usize = (SEQUENCE_REGION_START - INTRO_STREAM_START) as usize;
const SEQUENCE_REGION_SIZE: usize = (ENDING_REGION_END - SEQUENCE_REGION_START) as usize;
const GLYPH_FIRST_CODE: usize = 1;
const GLYPH_COUNT: usize = 60;
const TILE_SIZE: usize = 16;
const CHR_BANK_SIZE: usize = 1024;
const PAGE_BANKS: [usize; 4] = [0x2C, 0x80, 0x81, 0x82];
const PAGE_TABLE_SIZE: usize = 16;
const PAGE_TABLE_CPU: u16 = ENDING_REGION_END - PAGE_TABLE_SIZE as u16;
const SEQUENCE_CALL_CPU: u16 = 0x8261;
const FIXED_HOOK_CPU: u16 = 0xC020;
const EXPECTED_SEQUENCE_CALL: [u8; 3] = [0x20, 0x20, 0x8D];
const PATCHED_SEQUENCE_CALL: [u8; 3] = [0x20, 0x20, 0xC0];
const FIXED_PAGE_HOOK: [u8; 21] = [
    0x8A, 0x48, 0x98, 0x38, 0xE9, 0x39, 0x4A, 0xAA, 0xBD, 0x33, 0x84, 0x85, 0x83, 0x20, 0xBE, 0xED,
    0x68, 0xAA, 0x4C, 0x20, 0x8D,
];
const SEQUENCE_CLEAR_FIRST_ROW: u16 = 22;
const SEQUENCE_CLEAR_END_ROW_EXCLUSIVE: u16 = 24;

#[derive(Debug, Clone)]
pub struct EndingDraftPocReport {
    pub translation_status: String,
    pub entries: usize,
    pub unique_glyphs: usize,
    pub font_pages: usize,
    pub ending_used: usize,
    pub ending_remaining: usize,
}

#[derive(Debug, Clone)]
pub struct EndingDraftPocBuild {
    pub data: Vec<u8>,
    pub report: EndingDraftPocReport,
    pub writes: Vec<WriteReport>,
}

pub fn build(source: &Rom, spec: &EndingTranslation) -> Result<EndingDraftPocBuild> {
    let plan = plan(source, spec)?;
    validate_page_contract(&plan)?;
    let baseline = build_expanded_chr(source)?;
    let source_bank_start = ENDING_BANK * PRG_BANK_SIZE;
    let source_bank = &source.prg()[source_bank_start..source_bank_start + PRG_BANK_SIZE];
    let compiled = compile_ending(source_bank, &plan)?;
    let page_bytes = compile_font_pages(&plan)?;

    let mut image = TrackedImage::new(baseline.clone());
    let output_bank_start = HEADER_SIZE + source_bank_start;
    let ending_start = cpu_offset(INTRO_STREAM_START)?;
    let ending_end = cpu_offset(ENDING_REGION_END)?;
    image.write_expect(
        "install complete Korean ending translation",
        output_bank_start + ending_start,
        &source_bank[ending_start..ending_end],
        &compiled.bank[ending_start..ending_end],
    )?;
    let pointer_table = cpu_offset(POINTER_TABLE_CPU)?;
    for (index, _) in SEQUENCE_POINTER_ENTRIES {
        let offset = pointer_table + index as usize;
        image.write_expect(
            format!("relocate ending pointer {index:02X}"),
            output_bank_start + offset,
            &source_bank[offset..offset + 2],
            &compiled.bank[offset..offset + 2],
        )?;
    }
    let call_offset = cpu_offset(SEQUENCE_CALL_CPU)?;
    image.write_expect(
        "route ending sequence through CHR page selector",
        output_bank_start + call_offset,
        &EXPECTED_SEQUENCE_CALL,
        &PATCHED_SEQUENCE_CALL,
    )?;
    let fixed_hook_offset = HEADER_SIZE + 30 * PRG_BANK_SIZE + (FIXED_HOOK_CPU - 0xC000) as usize;
    image.write_expect(
        "install fixed CHR page selector",
        fixed_hook_offset,
        &[0xFF; FIXED_PAGE_HOOK.len()],
        &FIXED_PAGE_HOOK,
    )?;

    let chr_start = HEADER_SIZE + EXPANDED_PRG_SIZE;
    for (page, bytes) in page_bytes.iter().enumerate() {
        let bank = PAGE_BANKS[page];
        let offset = chr_start + bank * CHR_BANK_SIZE + GLYPH_FIRST_CODE * TILE_SIZE;
        let expected = if bank == 0x2C {
            let source_offset = bank * CHR_BANK_SIZE + GLYPH_FIRST_CODE * TILE_SIZE;
            source.chr()[source_offset..source_offset + GLYPH_COUNT * TILE_SIZE].to_vec()
        } else {
            vec![0; GLYPH_COUNT * TILE_SIZE]
        };
        image.write_expect(
            format!("install Korean font page {page} in CHR bank {bank:02X}"),
            offset,
            &expected,
            bytes,
        )?;
    }
    image.check_untracked_writes(&baseline)?;
    let writes = image.reports().to_vec();
    Ok(EndingDraftPocBuild {
        data: image.into_data(),
        report: EndingDraftPocReport {
            translation_status: plan.translation_status,
            entries: plan.entries.len(),
            unique_glyphs: plan.unique_glyphs,
            font_pages: plan.pages.len(),
            ending_used: compiled.used,
            ending_remaining: INTRO_REGION_SIZE + SEQUENCE_REGION_SIZE - compiled.used,
        },
        writes,
    })
}

struct CompiledEnding {
    bank: Vec<u8>,
    used: usize,
}

fn compile_ending(source_bank: &[u8], plan: &TranslationPlan) -> Result<CompiledEnding> {
    ensure!(
        source_bank.len() == PRG_BANK_SIZE,
        "P08 must be exactly 8 KiB"
    );
    let code_maps = page_code_maps(plan)?;
    let intro_source = parse_prefixed_asset(source_bank, INTRO_STREAM_START)?;
    ensure!(
        intro_source.records.len() == 5,
        "verified intro record count changed"
    );
    let intro_entry = &plan.entries[0];
    ensure!(
        intro_entry.draft_ko.len() == 3,
        "intro draft must use three lines"
    );

    let mut intro_records = encode_lines(
        &intro_entry.draft_ko,
        intro_entry.page,
        &code_maps,
        &[7, 11, 15],
    )?;
    intro_records.extend(
        intro_source.records[3..]
            .iter()
            .map(|record| ([record.coord0, record.coord1], record.tokens.clone())),
    );
    let intro = serialize_prefixed(&intro_records);
    ensure!(
        intro.len() <= INTRO_REGION_SIZE,
        "compiled intro uses {} bytes; envelope is {INTRO_REGION_SIZE}",
        intro.len()
    );

    let mut sequence = Vec::new();
    let mut relocated = BTreeMap::new();
    for entry in &plan.entries[1..] {
        relocated.insert(
            entry.asset_id.clone(),
            SEQUENCE_REGION_START + sequence.len() as u16,
        );
        let rows = sequence_rows(entry.draft_ko.len())?;
        let records = encode_lines(&entry.draft_ko, entry.page, &code_maps, &rows)?;
        sequence.extend_from_slice(&serialize_postfixed(&records));
    }
    ensure!(
        sequence.len() <= SEQUENCE_REGION_SIZE - PAGE_TABLE_SIZE,
        "compiled sequence uses {} bytes; text envelope is {}",
        sequence.len(),
        SEQUENCE_REGION_SIZE - PAGE_TABLE_SIZE
    );

    let mut bank = source_bank.to_vec();
    let intro_start = cpu_offset(INTRO_STREAM_START)?;
    bank[intro_start..intro_start + INTRO_REGION_SIZE].fill(0xFF);
    bank[intro_start..intro_start + intro.len()].copy_from_slice(&intro);
    let sequence_start = cpu_offset(SEQUENCE_REGION_START)?;
    bank[sequence_start..sequence_start + SEQUENCE_REGION_SIZE].fill(0xFF);
    bank[sequence_start..sequence_start + sequence.len()].copy_from_slice(&sequence);
    let page_table = cpu_offset(PAGE_TABLE_CPU)?;
    bank[page_table..page_table + PAGE_TABLE_SIZE].copy_from_slice(&compile_page_table(plan)?);
    let table = cpu_offset(POINTER_TABLE_CPU)?;
    for (index, old_pointer) in SEQUENCE_POINTER_ENTRIES {
        let first_index = SEQUENCE_POINTER_ENTRIES
            .iter()
            .find(|(_, pointer)| *pointer == old_pointer)
            .unwrap()
            .0;
        let asset_id = format!("ending/sequence-{first_index:02x}");
        let offset = table + index as usize;
        bank[offset..offset + 2].copy_from_slice(&relocated[&asset_id].to_le_bytes());
    }
    Ok(CompiledEnding {
        bank,
        used: intro.len() + sequence.len() + PAGE_TABLE_SIZE,
    })
}

fn sequence_rows(line_count: usize) -> Result<Vec<u16>> {
    let clear_rows =
        (SEQUENCE_CLEAR_FIRST_ROW..SEQUENCE_CLEAR_END_ROW_EXCLUSIVE).collect::<Vec<_>>();
    ensure!(
        (1..=clear_rows.len()).contains(&line_count),
        "sequence draft must fit the two-row clear window; three lines overwrite the star-box row"
    );
    Ok(clear_rows[clear_rows.len() - line_count..].to_vec())
}

fn page_code_maps(plan: &TranslationPlan) -> Result<Vec<BTreeMap<char, u8>>> {
    plan.pages
        .iter()
        .map(|page| {
            ensure!(
                page.glyph_count <= GLYPH_COUNT,
                "font page {} exceeds {GLYPH_COUNT} glyphs",
                page.page
            );
            Ok(page
                .glyphs
                .chars()
                .enumerate()
                .map(|(index, character)| (character, (index + 1) as u8))
                .collect())
        })
        .collect()
}

fn encode_lines(
    lines: &[String],
    page: usize,
    code_maps: &[BTreeMap<char, u8>],
    rows: &[u16],
) -> Result<Vec<([u8; 2], Vec<u8>)>> {
    ensure!(lines.len() == rows.len(), "line and row counts differ");
    lines
        .iter()
        .zip(rows)
        .map(|(line, row)| {
            let width = line.chars().count();
            let column = (32 - width) / 2;
            let address = 0x2000 + row * 32 + column as u16;
            let tokens =
                line.chars()
                    .map(|character| {
                        if character == ' ' {
                            Ok(0)
                        } else {
                            code_maps[page].get(&character).copied().ok_or_else(|| {
                                anyhow::anyhow!("missing page code for {character:?}")
                            })
                        }
                    })
                    .collect::<Result<Vec<_>>>()?;
            Ok((address.to_le_bytes(), tokens))
        })
        .collect()
}

fn serialize_prefixed(records: &[([u8; 2], Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![0xFF];
    for (index, (coords, tokens)) in records.iter().enumerate() {
        bytes.extend_from_slice(coords);
        bytes.extend_from_slice(tokens);
        bytes.push(if index + 1 == records.len() {
            0xFE
        } else {
            0xFF
        });
    }
    bytes
}

fn serialize_postfixed(records: &[([u8; 2], Vec<u8>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (index, (coords, tokens)) in records.iter().enumerate() {
        bytes.extend_from_slice(coords);
        bytes.extend_from_slice(tokens);
        bytes.push(if index + 1 == records.len() {
            0xFE
        } else {
            0xFF
        });
    }
    bytes
}

fn compile_font_pages(plan: &TranslationPlan) -> Result<Vec<Vec<u8>>> {
    plan.pages
        .iter()
        .map(|page| {
            let tiles = rasterize_dalmoori_text(&page.glyphs)?;
            let mut bytes = vec![0; GLYPH_COUNT * TILE_SIZE];
            for (index, tile) in tiles.iter().enumerate() {
                bytes[index * TILE_SIZE..(index + 1) * TILE_SIZE].copy_from_slice(tile);
            }
            Ok(bytes)
        })
        .collect()
}

fn validate_page_contract(plan: &TranslationPlan) -> Result<()> {
    ensure!(
        (1..=PAGE_BANKS.len()).contains(&plan.pages.len()),
        "draft PoC requires between one and {} font pages",
        PAGE_BANKS.len()
    );
    ensure!(
        plan.entries
            .iter()
            .all(|entry| entry.page < plan.pages.len()),
        "translation entry references an unavailable font page"
    );
    Ok(())
}

fn compile_page_table(plan: &TranslationPlan) -> Result<[u8; PAGE_TABLE_SIZE]> {
    let assignments = plan
        .entries
        .iter()
        .map(|entry| (entry.asset_id.as_str(), entry.page))
        .collect::<BTreeMap<_, _>>();
    page_table_from_assignments(&assignments)
}

fn page_table_from_assignments(
    assignments: &BTreeMap<&str, usize>,
) -> Result<[u8; PAGE_TABLE_SIZE]> {
    let mut table = [0; PAGE_TABLE_SIZE];
    for (position, (_, pointer)) in SEQUENCE_POINTER_ENTRIES.iter().enumerate() {
        let first_index = SEQUENCE_POINTER_ENTRIES
            .iter()
            .find(|(_, candidate)| candidate == pointer)
            .unwrap()
            .0;
        let asset_id = format!("ending/sequence-{first_index:02x}");
        let page = assignments
            .get(asset_id.as_str())
            .copied()
            .ok_or_else(|| anyhow::anyhow!("missing font page for {asset_id}"))?;
        table[position] = *PAGE_BANKS
            .get(page)
            .ok_or_else(|| anyhow::anyhow!("font page {page} exceeds the CHR bank budget"))?
            as u8;
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_hook_fits_the_proven_ff_cave_and_tail_calls_original_consumer() {
        assert_eq!(FIXED_PAGE_HOOK.len(), 21);
        assert_eq!(&FIXED_PAGE_HOOK[18..], &[0x4C, 0x20, 0x8D]);
    }

    #[test]
    fn font_pages_stop_before_the_diacritic_control_codes() {
        assert_eq!(GLYPH_FIRST_CODE + GLYPH_COUNT - 1, 0x3C);
    }

    #[test]
    fn centers_lines_in_the_bottom_nametable_rows() {
        let maps = vec![BTreeMap::from([('가', 1), ('나', 2)])];
        let encoded = encode_lines(&["가 나".to_owned()], 0, &maps, &[23]).unwrap();
        assert_eq!(encoded[0].0, 0x22EE_u16.to_le_bytes());
        assert_eq!(encoded[0].1, vec![1, 0, 2]);
    }

    #[test]
    fn sequence_lines_stay_inside_the_runtime_clear_window() {
        assert_eq!(sequence_rows(1).unwrap(), vec![23]);
        assert_eq!(sequence_rows(2).unwrap(), vec![22, 23]);
        assert!(sequence_rows(3).is_err());
    }

    #[test]
    fn selector_table_tracks_current_pages_and_pointer_aliases() {
        let assignments = BTreeMap::from([
            ("ending/sequence-38", 0),
            ("ending/sequence-3a", 0),
            ("ending/sequence-3c", 0),
            ("ending/sequence-3e", 0),
            ("ending/sequence-40", 0),
            ("ending/sequence-42", 1),
            ("ending/sequence-44", 1),
            ("ending/sequence-46", 1),
            ("ending/sequence-48", 1),
            ("ending/sequence-4a", 1),
            ("ending/sequence-4c", 1),
            ("ending/sequence-4e", 1),
            ("ending/sequence-50", 1),
        ]);

        let table = page_table_from_assignments(&assignments).unwrap();
        assert_eq!(&table[..5], &[0x2C; 5]);
        assert_eq!(&table[5..], &[0x80; 11]);
        assert_eq!(table[13], table[5]);
        assert_eq!(table[14], table[6]);
        assert_eq!(table[15], table[7]);
    }
}
