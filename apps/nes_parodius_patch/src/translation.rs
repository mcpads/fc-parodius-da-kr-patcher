use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{Context, Result, ensure};
use fontdue::{Font, FontSettings};
use serde::{Deserialize, Serialize};

use crate::{
    ending::translation_source_groups,
    font::rasterize_dalmoori,
    rom::{EXPECTED_SOURCE_SHA1, Rom},
};

const FORMAT_VERSION: u8 = 1;
const COMPLETE_STATUS: &str = "complete";
const MANUAL_TOKENS_VERIFIED: &str = "manual_and_tokens_verified";
const GLYPHS_PER_PAGE: usize = 60;
const MAX_LINE_TILES: usize = 28;
const DALMOORI_TTF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/dalmoori.ttf"
));

#[derive(Debug, Clone, Deserialize)]
pub struct EndingTranslation {
    format_version: u8,
    source_rom_sha1: String,
    source_reading_evidence: SourceReadingEvidence,
    translation_status: String,
    translation_review: TranslationReview,
    entries: Vec<TranslationEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SourceReadingEvidence {
    method: String,
    original_manual_scan_url: String,
    original_manual_sha256: String,
    manual_pdf_page: usize,
    manual_transcription_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct TranslationReview {
    decision: String,
    approved_by: String,
    approved_on: String,
    scope: String,
    basis: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TranslationEntry {
    asset_id: String,
    source_record_ids: Vec<String>,
    source_reading_status: String,
    #[serde(default)]
    source_ja: Vec<String>,
    reference_en: Vec<String>,
    draft_ko: Vec<String>,
    review_status: String,
}

#[derive(Debug, Serialize)]
pub struct TranslationPlan {
    format_version: u8,
    source_rom_sha1: String,
    source_reading_evidence: SourceReadingEvidence,
    pub translation_status: String,
    translation_review: TranslationReview,
    glyphs_per_page: usize,
    reserved_token_codes: [&'static str; 2],
    max_line_tiles: usize,
    pub unique_glyphs: usize,
    pub entries: Vec<EntryPlan>,
    pub pages: Vec<PagePlan>,
}

#[derive(Debug, Serialize)]
pub struct EntryPlan {
    pub(crate) asset_id: String,
    source_record_ids: Vec<String>,
    source_reading_status: String,
    source_reading_lines: usize,
    reference_lines: usize,
    review_status: String,
    pub(crate) draft_ko: Vec<String>,
    line_tiles: Vec<usize>,
    unique_glyphs: usize,
    pub(crate) page: usize,
}

#[derive(Debug, Serialize)]
pub struct PagePlan {
    pub(crate) page: usize,
    entries: Vec<String>,
    pub(crate) glyph_count: usize,
    pub(crate) glyphs: String,
}

impl EndingTranslation {
    pub fn from_path(path: &Path) -> Result<Self> {
        let data = fs::read(path)
            .with_context(|| format!("read ending translation {}", path.display()))?;
        serde_json::from_slice(&data)
            .with_context(|| format!("parse ending translation {}", path.display()))
    }
}

pub fn plan(source: &Rom, spec: &EndingTranslation) -> Result<TranslationPlan> {
    source.verify_supported_japanese()?;
    ensure!(
        spec.format_version == FORMAT_VERSION,
        "translation format_version must be {FORMAT_VERSION}"
    );
    ensure!(
        spec.source_rom_sha1 == EXPECTED_SOURCE_SHA1,
        "translation source_rom_sha1 must match the supported Japanese ROM"
    );
    ensure!(
        spec.source_reading_evidence.method
            == "original_manual_lyrics_cross_checked_against_rom_tokens",
        "translation source-reading evidence method is unsupported"
    );
    ensure!(
        spec.source_reading_evidence
            .original_manual_scan_url
            .starts_with("https://")
            && spec
                .source_reading_evidence
                .manual_transcription_url
                .starts_with("https://"),
        "translation source-reading evidence URLs must use HTTPS"
    );
    ensure!(
        spec.source_reading_evidence.original_manual_sha256.len() == 64
            && spec
                .source_reading_evidence
                .original_manual_sha256
                .chars()
                .all(|character| character.is_ascii_hexdigit()),
        "translation original-manual SHA-256 must be 64 hexadecimal characters"
    );
    ensure!(
        spec.source_reading_evidence.manual_pdf_page > 0,
        "translation manual PDF page must be one-based"
    );
    ensure!(
        spec.translation_status == COMPLETE_STATUS,
        "translation_status must be {COMPLETE_STATUS} after project-owner approval"
    );
    validate_translation_review(&spec.translation_review)?;

    let sources = translation_source_groups(source)?;
    ensure!(
        spec.entries.len() == sources.len(),
        "translation entry count does not match the Japanese ending population"
    );
    let font = Font::from_bytes(DALMOORI_TTF, FontSettings::default())
        .map_err(|error| anyhow::anyhow!("load vendored Dalmoori font: {error}"))?;
    let mut seen_assets = BTreeSet::new();
    let mut global_glyphs: BTreeSet<char> = BTreeSet::new();
    let mut prepared = Vec::new();

    for (entry, source_group) in spec.entries.iter().zip(&sources) {
        ensure!(
            entry.asset_id == source_group.asset_id,
            "translation asset order mismatch: expected {}, found {}",
            source_group.asset_id,
            entry.asset_id
        );
        ensure!(
            seen_assets.insert(entry.asset_id.clone()),
            "duplicate translation asset {}",
            entry.asset_id
        );
        ensure!(
            entry.source_record_ids == source_group.record_ids,
            "translation source records do not match exact Japanese asset {}",
            entry.asset_id
        );
        validate_source_reading(entry, source_group)?;
        ensure!(
            !entry.reference_en.is_empty(),
            "reference translation is empty for {}",
            entry.asset_id
        );
        ensure!(
            entry.review_status == COMPLETE_STATUS,
            "entry {} must be {COMPLETE_STATUS}",
            entry.asset_id
        );
        ensure!(
            !entry.draft_ko.is_empty(),
            "Korean draft is empty for {}",
            entry.asset_id
        );

        let mut glyphs = BTreeSet::new();
        let mut line_tiles = Vec::new();
        for line in &entry.draft_ko {
            ensure!(
                !line.is_empty() && line.trim() == line,
                "Korean draft line has leading/trailing whitespace in {}",
                entry.asset_id
            );
            let width = line.chars().count();
            ensure!(
                width <= MAX_LINE_TILES,
                "Korean draft line in {} uses {width} tiles; limit is {MAX_LINE_TILES}",
                entry.asset_id
            );
            line_tiles.push(width);
            for character in line.chars().filter(|character| *character != ' ') {
                glyphs.insert(character);
            }
        }
        ensure!(
            glyphs.len() <= GLYPHS_PER_PAGE,
            "asset {} alone needs {} glyphs; one page holds {GLYPHS_PER_PAGE}",
            entry.asset_id,
            glyphs.len()
        );
        for character in &glyphs {
            rasterize_dalmoori(&font, *character)?;
        }
        global_glyphs.extend(glyphs.iter().copied());
        prepared.push((entry, line_tiles, glyphs));
    }

    let (page_sets, assignments) = partition_pages(&prepared);
    let entries = prepared
        .iter()
        .zip(assignments)
        .map(|((entry, line_tiles, glyphs), page)| EntryPlan {
            asset_id: entry.asset_id.clone(),
            source_record_ids: entry.source_record_ids.clone(),
            source_reading_status: entry.source_reading_status.clone(),
            source_reading_lines: entry.source_ja.len(),
            reference_lines: entry.reference_en.len(),
            review_status: entry.review_status.clone(),
            draft_ko: entry.draft_ko.clone(),
            line_tiles: line_tiles.clone(),
            unique_glyphs: glyphs.len(),
            page,
        })
        .collect::<Vec<_>>();
    let mut entries_by_page: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for entry in &entries {
        entries_by_page
            .entry(entry.page)
            .or_default()
            .push(entry.asset_id.clone());
    }
    let pages = page_sets
        .into_iter()
        .enumerate()
        .map(|(page, glyphs)| PagePlan {
            page,
            entries: entries_by_page.remove(&page).unwrap(),
            glyph_count: glyphs.len(),
            glyphs: glyphs.into_iter().collect(),
        })
        .collect();

    Ok(TranslationPlan {
        format_version: FORMAT_VERSION,
        source_rom_sha1: EXPECTED_SOURCE_SHA1.to_owned(),
        source_reading_evidence: spec.source_reading_evidence.clone(),
        translation_status: spec.translation_status.clone(),
        translation_review: spec.translation_review.clone(),
        glyphs_per_page: GLYPHS_PER_PAGE,
        reserved_token_codes: ["0x3D", "0x3E"],
        max_line_tiles: MAX_LINE_TILES,
        unique_glyphs: global_glyphs.len(),
        entries,
        pages,
    })
}

fn validate_translation_review(review: &TranslationReview) -> Result<()> {
    ensure!(
        review.decision == "approved",
        "translation review decision must be approved"
    );
    ensure!(
        review.approved_by == "project_owner",
        "translation review must be approved by project_owner"
    );
    ensure!(
        review.scope == "all_14_ending_assets",
        "translation review scope must cover all 14 ending assets"
    );
    ensure!(
        review.basis == "source_reading_and_integrated_runtime_review",
        "translation review basis is unsupported"
    );
    let date = review.approved_on.as_bytes();
    ensure!(
        date.len() == 10
            && date[4] == b'-'
            && date[7] == b'-'
            && date
                .iter()
                .enumerate()
                .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit()),
        "translation review approved_on must use YYYY-MM-DD"
    );
    Ok(())
}

fn validate_source_reading(
    entry: &TranslationEntry,
    source_group: &crate::ending::TranslationSourceGroup,
) -> Result<()> {
    ensure!(
        matches!(
            entry.source_reading_status.as_str(),
            "runtime_verified" | MANUAL_TOKENS_VERIFIED | "tokens_verified_reading_pending"
        ),
        "unsupported source_reading_status for {}",
        entry.asset_id
    );
    ensure!(
        entry.source_reading_status == "tokens_verified_reading_pending"
            || !entry.source_ja.is_empty(),
        "verified source reading is empty for {}",
        entry.asset_id
    );
    if entry.source_reading_status == MANUAL_TOKENS_VERIFIED {
        ensure!(
            entry.source_ja.len() == source_group.text_token_counts.len(),
            "manual/token source line count mismatch for {}",
            entry.asset_id
        );
        for (line, token_count) in entry.source_ja.iter().zip(&source_group.text_token_counts) {
            ensure!(
                line.chars().count() == *token_count,
                "manual/token source length mismatch for {}: reading has {} characters, ROM has {token_count} tokens",
                entry.asset_id,
                line.chars().count()
            );
        }
    }
    Ok(())
}

fn partition_pages(
    entries: &[(&TranslationEntry, Vec<usize>, BTreeSet<char>)],
) -> (Vec<BTreeSet<char>>, Vec<usize>) {
    let mut pages = Vec::new();
    let mut assignments = Vec::new();
    let mut current = BTreeSet::new();
    for (_, _, glyphs) in entries {
        let combined = current.union(glyphs).copied().collect::<BTreeSet<_>>();
        if !current.is_empty() && combined.len() > GLYPHS_PER_PAGE {
            pages.push(current);
            current = glyphs.clone();
        } else {
            current = combined;
        }
        assignments.push(pages.len());
    }
    if !current.is_empty() {
        pages.push(current);
    }
    (pages, assignments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(asset_id: &str, text: &str) -> TranslationEntry {
        TranslationEntry {
            asset_id: asset_id.to_owned(),
            source_record_ids: vec![],
            source_reading_status: "tokens_verified_reading_pending".to_owned(),
            source_ja: vec![],
            reference_en: vec!["reference".to_owned()],
            draft_ko: vec![text.to_owned()],
            review_status: COMPLETE_STATUS.to_owned(),
        }
    }

    #[test]
    fn keeps_each_asset_wholly_inside_one_font_page() {
        let first = entry("first", &"가".repeat(2));
        let second = entry("second", "나다");
        let prepared = vec![
            (&first, vec![2], BTreeSet::from(['가'])),
            (&second, vec![2], BTreeSet::from(['나', '다'])),
        ];
        let (pages, assignments) = partition_pages(&prepared);
        assert_eq!(pages.len(), 1);
        assert_eq!(assignments, vec![0, 0]);
        assert_eq!(pages[0], BTreeSet::from(['가', '나', '다']));
    }

    #[test]
    fn manual_source_reading_must_match_the_rom_token_count() {
        let mut entry = entry("ending/sequence-test", "번역");
        entry.source_reading_status = MANUAL_TOKENS_VERIFIED.to_owned();
        entry.source_ja = vec!["てすと".to_owned()];
        let source = crate::ending::TranslationSourceGroup {
            asset_id: entry.asset_id.clone(),
            record_ids: vec!["ending/sequence-test/record-00".to_owned()],
            text_token_counts: vec![2],
        };

        let error = validate_source_reading(&entry, &source).unwrap_err();
        assert!(error.to_string().contains("ROM has 2 tokens"));
    }

    #[test]
    fn complete_translation_requires_project_owner_approval_evidence() {
        let approved = TranslationReview {
            decision: "approved".to_owned(),
            approved_by: "project_owner".to_owned(),
            approved_on: "2026-07-14".to_owned(),
            scope: "all_14_ending_assets".to_owned(),
            basis: "source_reading_and_integrated_runtime_review".to_owned(),
        };
        validate_translation_review(&approved).unwrap();

        let invalid = TranslationReview {
            approved_by: "unreviewed_agent".to_owned(),
            ..approved
        };
        assert!(validate_translation_review(&invalid).is_err());
    }
}
