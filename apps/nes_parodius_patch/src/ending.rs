use std::collections::BTreeMap;

use anyhow::{Result, bail, ensure};
use serde::Serialize;

use crate::{
    rom::{PRG_BANK_SIZE, Rom},
    sha1_hex,
};

pub const ENDING_BANK: usize = 8;
pub const POINTER_TABLE_CPU: u16 = 0x8F32;
pub const INTRO_POINTER_STORAGE_CPU: u16 = 0x828B;
pub const INTRO_STREAM_START: u16 = 0x828D;
pub const SEQUENCE_REGION_START: u16 = 0x82F3;
pub const SONG_REGION_START: u16 = 0x8355;
pub const ENDING_REGION_END: u16 = 0x8443;

pub const SEQUENCE_POINTER_ENTRIES: [(u8, u16); 16] = [
    (0x38, 0x82F3),
    (0x3A, 0x8304),
    (0x3C, 0x830E),
    (0x3E, 0x8328),
    (0x40, 0x8355),
    (0x42, 0x8368),
    (0x44, 0x8388),
    (0x46, 0x839D),
    (0x48, 0x83C8),
    (0x4A, 0x83DB),
    (0x4C, 0x83EA),
    (0x4E, 0x8410),
    (0x50, 0x842D),
    (0x52, 0x8368),
    (0x54, 0x8388),
    (0x56, 0x839D),
];

pub const SONG_POINTER_ENTRIES: [(u8, u16); 12] = [
    (0x40, 0x8355),
    (0x42, 0x8368),
    (0x44, 0x8388),
    (0x46, 0x839D),
    (0x48, 0x83C8),
    (0x4A, 0x83DB),
    (0x4C, 0x83EA),
    (0x4E, 0x8410),
    (0x50, 0x842D),
    (0x52, 0x8368),
    (0x54, 0x8388),
    (0x56, 0x839D),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub coord0: u8,
    pub coord1: u8,
    pub tokens: Vec<u8>,
    pub terminator: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Asset {
    pub pointer: u16,
    pub end: u16,
    pub records: Vec<Record>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranslationSourceGroup {
    pub asset_id: String,
    pub record_ids: Vec<String>,
    pub text_token_counts: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub struct CorpusManifest {
    format_version: u8,
    source: SourceManifest,
    region: RegionManifest,
    totals: TotalsManifest,
    intro: AssetManifest,
    pointer_entries: Vec<PointerManifest>,
    sequence_assets: Vec<AssetManifest>,
}

#[derive(Debug, Serialize)]
struct SourceManifest {
    rom_sha1: String,
    physical_prg_bank: String,
}

#[derive(Debug, Serialize)]
struct RegionManifest {
    intro_pointer_storage_cpu: String,
    intro_stream_start_cpu: String,
    sequence_start_cpu: String,
    end_cpu_exclusive: String,
    size: usize,
    pointer_table_cpu: String,
}

#[derive(Debug, Serialize)]
struct TotalsManifest {
    unique_assets: usize,
    records: usize,
    translation_text_records: usize,
    diacritic_overlay_records: usize,
    protected_existing_latin_records: usize,
    pointer_entries: usize,
}

#[derive(Debug, Serialize)]
struct PointerManifest {
    reference_id: String,
    normalized_index: String,
    storage_cpu: String,
    raw_hex: String,
    target_cpu: String,
    asset_id: String,
}

#[derive(Debug, Serialize)]
struct AssetManifest {
    asset_id: String,
    record_format: &'static str,
    start_cpu: String,
    end_cpu_exclusive: String,
    size: usize,
    raw_hex: String,
    records: Vec<RecordManifest>,
}

#[derive(Debug, Serialize)]
struct RecordManifest {
    record_id: String,
    coord0: String,
    coord1: String,
    tokens_hex: String,
    terminator: String,
    classification: &'static str,
}

pub fn extract_manifest(source: &Rom) -> Result<CorpusManifest> {
    source.verify_supported_japanese()?;
    let bank_start = ENDING_BANK * PRG_BANK_SIZE;
    let bank = &source.prg()[bank_start..bank_start + PRG_BANK_SIZE];
    validate_pointer_entries(bank, &SEQUENCE_POINTER_ENTRIES)?;

    let pointer_storage = cpu_offset(INTRO_POINTER_STORAGE_CPU)?;
    let intro_pointer = u16::from_le_bytes(
        bank[pointer_storage..pointer_storage + 2]
            .try_into()
            .unwrap(),
    );
    ensure!(
        intro_pointer == INTRO_STREAM_START,
        "intro pointer: expected {INTRO_STREAM_START:04X}, found {intro_pointer:04X}"
    );
    let intro = parse_prefixed_asset(bank, intro_pointer)?;
    ensure!(
        intro.end == SEQUENCE_REGION_START,
        "intro ends at {:04X}, expected {SEQUENCE_REGION_START:04X}",
        intro.end
    );

    let sequence = unique_assets(bank, &SEQUENCE_POINTER_ENTRIES)?;
    ensure_contiguous(&sequence, SEQUENCE_REGION_START, ENDING_REGION_END)?;
    verify_roundtrip(bank, &intro, &sequence)?;

    let mut ids = BTreeMap::new();
    for (index, pointer) in SEQUENCE_POINTER_ENTRIES {
        ids.entry(pointer)
            .or_insert_with(|| format!("ending/sequence-{index:02x}"));
    }
    let intro_manifest = asset_manifest(
        "ending/intro",
        "ff-prefixed-coordinate-records",
        bank,
        &intro,
        &intro_record_roles(&intro)?,
    )?;
    let sequence_manifests: Vec<_> = sequence
        .values()
        .map(|asset| {
            let roles = sequence_record_roles(asset)?;
            asset_manifest(
                &ids[&asset.pointer],
                "coordinate-records-with-ff-fe-terminators",
                bank,
                asset,
                &roles,
            )
        })
        .collect::<Result<_>>()?;
    let pointer_entries: Vec<_> = SEQUENCE_POINTER_ENTRIES
        .iter()
        .map(|(index, pointer)| {
            let storage = POINTER_TABLE_CPU + *index as u16;
            PointerManifest {
                reference_id: format!("ending/ref-{index:02x}"),
                normalized_index: hex_u8(*index),
                storage_cpu: hex_u16(storage),
                raw_hex: format!(
                    "{:02X} {:02X}",
                    pointer.to_le_bytes()[0],
                    pointer.to_le_bytes()[1]
                ),
                target_cpu: hex_u16(*pointer),
                asset_id: ids[pointer].clone(),
            }
        })
        .collect();

    let records = std::iter::once(&intro_manifest)
        .chain(&sequence_manifests)
        .flat_map(|asset| &asset.records);
    let (mut translation_text_records, mut diacritic_overlay_records) = (0, 0);
    let mut protected_existing_latin_records = 0;
    let mut record_count = 0;
    for record in records {
        record_count += 1;
        match record.classification {
            "translation_text" => translation_text_records += 1,
            "diacritic_overlay" => diacritic_overlay_records += 1,
            "protected_existing_latin" => protected_existing_latin_records += 1,
            _ => unreachable!("all record roles are assigned by the extractor"),
        }
    }

    Ok(CorpusManifest {
        format_version: 1,
        source: SourceManifest {
            rom_sha1: sha1_hex(source.data()),
            physical_prg_bank: "P08".to_owned(),
        },
        region: RegionManifest {
            intro_pointer_storage_cpu: hex_u16(INTRO_POINTER_STORAGE_CPU),
            intro_stream_start_cpu: hex_u16(INTRO_STREAM_START),
            sequence_start_cpu: hex_u16(SEQUENCE_REGION_START),
            end_cpu_exclusive: hex_u16(ENDING_REGION_END),
            size: (ENDING_REGION_END - INTRO_POINTER_STORAGE_CPU) as usize,
            pointer_table_cpu: hex_u16(POINTER_TABLE_CPU),
        },
        totals: TotalsManifest {
            unique_assets: sequence_manifests.len() + 1,
            records: record_count,
            translation_text_records,
            diacritic_overlay_records,
            protected_existing_latin_records,
            pointer_entries: pointer_entries.len(),
        },
        intro: intro_manifest,
        pointer_entries,
        sequence_assets: sequence_manifests,
    })
}

pub(crate) fn translation_source_groups(source: &Rom) -> Result<Vec<TranslationSourceGroup>> {
    let manifest = extract_manifest(source)?;
    let assets = std::iter::once(manifest.intro)
        .chain(manifest.sequence_assets)
        .map(|asset| TranslationSourceGroup {
            asset_id: asset.asset_id,
            record_ids: asset
                .records
                .iter()
                .filter(|record| record.classification == "translation_text")
                .map(|record| record.record_id.clone())
                .collect(),
            text_token_counts: asset
                .records
                .iter()
                .filter(|record| record.classification == "translation_text")
                .map(|record| record.tokens_hex.split_ascii_whitespace().count())
                .collect(),
        })
        .collect::<Vec<_>>();
    ensure!(
        assets.iter().all(|asset| !asset.record_ids.is_empty()),
        "every ending asset must expose at least one translation record"
    );
    Ok(assets)
}

pub(crate) fn source_song_assets(bank: &[u8]) -> Result<BTreeMap<u16, Asset>> {
    validate_pointer_entries(bank, &SONG_POINTER_ENTRIES)?;
    let assets = unique_assets(bank, &SONG_POINTER_ENTRIES)?;
    ensure_contiguous(&assets, SONG_REGION_START, ENDING_REGION_END)?;
    Ok(assets)
}

fn validate_pointer_entries(bank: &[u8], entries: &[(u8, u16)]) -> Result<()> {
    ensure!(bank.len() == PRG_BANK_SIZE, "P08 must be exactly 8 KiB");
    let table = cpu_offset(POINTER_TABLE_CPU)?;
    for (index, expected_pointer) in entries {
        let offset = table + *index as usize;
        let actual = u16::from_le_bytes(bank[offset..offset + 2].try_into().unwrap());
        ensure!(
            actual == *expected_pointer,
            "ending pointer {index:02X}: expected {expected_pointer:04X}, found {actual:04X}"
        );
    }
    Ok(())
}

fn unique_assets(bank: &[u8], entries: &[(u8, u16)]) -> Result<BTreeMap<u16, Asset>> {
    let mut assets = BTreeMap::new();
    for (_, pointer) in entries {
        if let std::collections::btree_map::Entry::Vacant(entry) = assets.entry(*pointer) {
            entry.insert(parse_postfixed_asset(bank, *pointer)?);
        }
    }
    Ok(assets)
}

fn ensure_contiguous(
    assets: &BTreeMap<u16, Asset>,
    expected_start: u16,
    expected_end: u16,
) -> Result<()> {
    let mut next = expected_start;
    for asset in assets.values() {
        ensure!(
            asset.pointer == next,
            "ending assets are not contiguous at {:04X}",
            asset.pointer
        );
        next = asset.end;
    }
    ensure!(
        next == expected_end,
        "ending envelope ends at {next:04X}, expected {expected_end:04X}"
    );
    Ok(())
}

pub(crate) fn parse_prefixed_asset(bank: &[u8], pointer: u16) -> Result<Asset> {
    let mut position = cpu_offset(pointer)?;
    let start = position;
    let mut records = Vec::new();
    loop {
        ensure!(
            position < bank.len() && bank[position] == 0xFF,
            "intro record at {:04X} is missing its FF prefix",
            0x8000 + position as u16
        );
        position += 1;
        ensure!(
            position + 2 <= bank.len(),
            "intro record at {pointer:04X} has truncated coordinates"
        );
        let coord0 = bank[position];
        let coord1 = bank[position + 1];
        position += 2;
        let token_start = position;
        while position < bank.len() && !matches!(bank[position], 0xFE | 0xFF) {
            position += 1;
        }
        ensure!(position < bank.len(), "intro asset has no terminator");
        let terminator = bank[position];
        records.push(Record {
            coord0,
            coord1,
            tokens: bank[token_start..position].to_vec(),
            terminator,
        });
        if terminator == 0xFE {
            position += 1;
            return Ok(Asset {
                pointer,
                end: 0x8000 + position as u16,
                records,
            });
        }
        ensure!(
            position - start <= (ENDING_REGION_END - INTRO_STREAM_START) as usize,
            "intro asset exceeds the ending envelope"
        );
    }
}

pub(crate) fn parse_postfixed_asset(bank: &[u8], pointer: u16) -> Result<Asset> {
    let mut position = cpu_offset(pointer)?;
    let start = position;
    let mut records = Vec::new();
    while position < bank.len() {
        ensure!(
            position + 2 <= bank.len(),
            "asset {pointer:04X} has truncated coordinates"
        );
        let coord0 = bank[position];
        let coord1 = bank[position + 1];
        position += 2;
        let token_start = position;
        while position < bank.len() && !matches!(bank[position], 0xFE | 0xFF) {
            position += 1;
        }
        ensure!(
            position < bank.len(),
            "asset {pointer:04X} has no terminator"
        );
        let terminator = bank[position];
        records.push(Record {
            coord0,
            coord1,
            tokens: bank[token_start..position].to_vec(),
            terminator,
        });
        position += 1;
        if terminator == 0xFE {
            return Ok(Asset {
                pointer,
                end: 0x8000 + position as u16,
                records,
            });
        }
        ensure!(
            position - start <= (ENDING_REGION_END - SEQUENCE_REGION_START) as usize,
            "asset {pointer:04X} exceeds the ending envelope"
        );
    }
    bail!("asset {pointer:04X} exceeded P08")
}

fn verify_roundtrip(bank: &[u8], intro: &Asset, sequence: &BTreeMap<u16, Asset>) -> Result<()> {
    let intro_bytes = serialize_prefixed(intro);
    let intro_start = cpu_offset(intro.pointer)?;
    let intro_end = cpu_offset(intro.end)?;
    ensure!(
        intro_bytes == bank[intro_start..intro_end],
        "intro parse/serialize roundtrip mismatch"
    );
    for asset in sequence.values() {
        let bytes = serialize_postfixed(asset);
        let start = cpu_offset(asset.pointer)?;
        let end = cpu_offset(asset.end)?;
        ensure!(
            bytes == bank[start..end],
            "asset {:04X} parse/serialize roundtrip mismatch",
            asset.pointer
        );
    }
    Ok(())
}

fn serialize_prefixed(asset: &Asset) -> Vec<u8> {
    let mut output = vec![0xFF];
    for (record_number, record) in asset.records.iter().enumerate() {
        output.extend_from_slice(&[record.coord0, record.coord1]);
        output.extend_from_slice(&record.tokens);
        if record_number + 1 == asset.records.len() {
            output.push(0xFE);
        } else {
            output.push(0xFF);
        }
    }
    output
}

fn serialize_postfixed(asset: &Asset) -> Vec<u8> {
    let mut output = Vec::new();
    for record in &asset.records {
        output.extend_from_slice(&[record.coord0, record.coord1]);
        output.extend_from_slice(&record.tokens);
        output.push(record.terminator);
    }
    output
}

fn asset_manifest(
    asset_id: &str,
    record_format: &'static str,
    bank: &[u8],
    asset: &Asset,
    roles: &[&'static str],
) -> Result<AssetManifest> {
    ensure!(
        roles.len() == asset.records.len(),
        "asset {asset_id} role count does not match its record count"
    );
    let start = cpu_offset(asset.pointer)?;
    let end = cpu_offset(asset.end)?;
    let records = asset
        .records
        .iter()
        .enumerate()
        .map(|(record_number, record)| RecordManifest {
            record_id: format!("{asset_id}/record-{record_number:02}"),
            coord0: hex_u8(record.coord0),
            coord1: hex_u8(record.coord1),
            tokens_hex: hex_bytes(&record.tokens),
            terminator: hex_u8(record.terminator),
            classification: roles[record_number],
        })
        .collect();
    Ok(AssetManifest {
        asset_id: asset_id.to_owned(),
        record_format,
        start_cpu: hex_u16(asset.pointer),
        end_cpu_exclusive: hex_u16(asset.end),
        size: end - start,
        raw_hex: hex_bytes(&bank[start..end]),
        records,
    })
}

fn intro_record_roles(asset: &Asset) -> Result<Vec<&'static str>> {
    ensure!(
        asset.records.len() == 5,
        "verified intro must contain three Japanese lines and two existing Latin lines"
    );
    Ok(vec![
        "translation_text",
        "translation_text",
        "translation_text",
        "protected_existing_latin",
        "protected_existing_latin",
    ])
}

fn sequence_record_roles(asset: &Asset) -> Result<Vec<&'static str>> {
    ensure!(
        !asset.records.is_empty(),
        "ending sequence asset {:04X} has no records",
        asset.pointer
    );
    let last = asset.records.len() - 1;
    for record in &asset.records[..last] {
        ensure!(
            !record.tokens.is_empty()
                && record
                    .tokens
                    .iter()
                    .all(|token| matches!(token, 0x00 | 0x3D | 0x3E)),
            "ending sequence asset {:04X} has a non-diacritic prefix record",
            asset.pointer
        );
    }
    let mut roles = vec!["diacritic_overlay"; last];
    roles.push("translation_text");
    Ok(roles)
}

pub(crate) fn cpu_offset(address: u16) -> Result<usize> {
    ensure!(
        (0x8000..=0x9FFF).contains(&address),
        "CPU address {address:04X} is outside the P08 switchable window"
    );
    Ok((address - 0x8000) as usize)
}

fn hex_u8(value: u8) -> String {
    format!("0x{value:02X}")
}

fn hex_u16(value: u16) -> String {
    format!("0x{value:04X}")
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_both_ending_record_formats() {
        let mut bank = vec![0; PRG_BANK_SIZE];
        let prefixed = [0xFF, 0xE5, 0x20, 0x14, 0x03, 0xFF, 0x05, 0x21, 0x15, 0xFE];
        let prefixed_start = cpu_offset(0x828D).unwrap();
        bank[prefixed_start..prefixed_start + prefixed.len()].copy_from_slice(&prefixed);
        let intro = parse_prefixed_asset(&bank, 0x828D).unwrap();
        assert_eq!(serialize_prefixed(&intro), prefixed);
        assert_eq!(intro.records.len(), 2);

        let postfixed = [0xE4, 0x22, 0x01, 0x02, 0xFF, 0x04, 0x23, 0x03, 0xFE];
        let postfixed_start = cpu_offset(0x8355).unwrap();
        bank[postfixed_start..postfixed_start + postfixed.len()].copy_from_slice(&postfixed);
        let asset = parse_postfixed_asset(&bank, 0x8355).unwrap();
        assert_eq!(serialize_postfixed(&asset), postfixed);
        assert_eq!(asset.records.len(), 2);
    }

    #[test]
    fn classifies_sequence_structure_separately_from_token_values() {
        let asset = Asset {
            pointer: 0x8355,
            end: 0x8360,
            records: vec![
                Record {
                    coord0: 0,
                    coord1: 0,
                    tokens: vec![0x3D, 0, 0x3E],
                    terminator: 0xFF,
                },
                Record {
                    coord0: 0,
                    coord1: 0,
                    tokens: vec![0x01, 0xB7, 0xA9],
                    terminator: 0xFE,
                },
            ],
        };
        assert_eq!(
            sequence_record_roles(&asset).unwrap(),
            vec!["diacritic_overlay", "translation_text"]
        );
    }
}
