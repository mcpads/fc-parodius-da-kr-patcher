use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::rom::{EXPECTED_SOURCE_SHA1, Rom};

const FORMAT_VERSION: u8 = 5;
const COMPLETE_STATUS: &str = "complete";
const ROULETTE_LABEL_ASSET_ID: &str = "baked/roulette-label";
const ROULETTE_LABEL_SOURCE_JA: &str = "ルーレット";
const ROULETTE_LABEL_REFERENCE_EN: &str = "ROULETTE";
const ROULETTE_LABEL_SOURCE_STATUS: &str = "static_metasprite_and_project_owner_reading_verified";
const HIT_LABEL_ASSET_ID: &str = "baked/hit-label";
const HIT_LABEL_SOURCE_JA: &str = "当";
const HIT_LABEL_REFERENCE_EN: &str = "HIT";
const RIP_MARKER_ASSET_ID: &str = "baked/rip-marker";
const RIP_MARKER_SOURCE_JA: &str = "南無";
const RIP_MARKER_REFERENCE_EN: &str = "RIP";
const STATIC_TILE_LAYOUT_STATUS: &str = "static_tile_layout_verified";
const NEXON_ROULETTE_RENDER_STRATEGY: &str =
    "nexon_lv2_gothic_13px_threshold96_baked_tritone_contiguous_oam";
const REFERENCE_PIXEL_RENDER_STRATEGY: &str = "reference_pixel_delta";

#[derive(Debug, Clone, Deserialize)]
pub struct GraphicsTranslation {
    format_version: u8,
    source_rom_sha1: String,
    translation_status: String,
    translation_review: TranslationReview,
    entries: Vec<GraphicsTranslationEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct TranslationReview {
    decision: String,
    approved_by: String,
    approved_on: String,
    scope: String,
    basis: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphicsTranslationEntry {
    asset_id: String,
    source_reading_status: String,
    source_ja: String,
    reference_en: String,
    target_text: String,
    target_language: String,
    render_strategy: String,
    review_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouletteLabelTranslation {
    pub source_ja: String,
    pub reference_en: String,
    pub text: String,
    pub translation_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceLabelTranslation {
    pub asset_id: String,
    pub source_ja: String,
    pub reference_en: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceLabelsTranslation {
    pub hit: ReferenceLabelTranslation,
    pub rip: ReferenceLabelTranslation,
    pub translation_status: String,
}

impl GraphicsTranslation {
    pub fn from_path(path: &Path) -> Result<Self> {
        let data = fs::read(path)
            .with_context(|| format!("read graphics translation {}", path.display()))?;
        serde_json::from_slice(&data)
            .with_context(|| format!("parse graphics translation {}", path.display()))
    }

    pub fn validate_roulette_label(&self, source: &Rom) -> Result<RouletteLabelTranslation> {
        self.validate_common(source)?;
        let entry = self.entry(ROULETTE_LABEL_ASSET_ID)?;
        ensure!(
            entry.source_reading_status == ROULETTE_LABEL_SOURCE_STATUS,
            "roulette-label source reading must be verified from the static metasprite and project-owner reading"
        );
        ensure!(
            entry.source_ja == ROULETTE_LABEL_SOURCE_JA,
            "roulette-label Japanese source must be {ROULETTE_LABEL_SOURCE_JA}"
        );
        ensure!(
            entry.reference_en == ROULETTE_LABEL_REFERENCE_EN,
            "roulette-label English reference must be {ROULETTE_LABEL_REFERENCE_EN}"
        );
        ensure!(
            entry.review_status == COMPLETE_STATUS,
            "roulette-label review_status must be {COMPLETE_STATUS}"
        );
        ensure!(
            entry.target_language == "ko",
            "roulette-label target_language must be ko"
        );
        ensure!(
            entry.render_strategy == NEXON_ROULETTE_RENDER_STRATEGY,
            "roulette-label render_strategy must be {NEXON_ROULETTE_RENDER_STRATEGY}"
        );
        validate_roulette_label_text(&entry.target_text)?;

        Ok(RouletteLabelTranslation {
            source_ja: entry.source_ja.clone(),
            reference_en: entry.reference_en.clone(),
            text: entry.target_text.clone(),
            translation_status: self.translation_status.clone(),
        })
    }

    pub fn validate_reference_labels(&self, source: &Rom) -> Result<ReferenceLabelsTranslation> {
        self.validate_common(source)?;
        let hit = self.validate_reference_label(
            HIT_LABEL_ASSET_ID,
            HIT_LABEL_SOURCE_JA,
            HIT_LABEL_REFERENCE_EN,
        )?;
        let rip = self.validate_reference_label(
            RIP_MARKER_ASSET_ID,
            RIP_MARKER_SOURCE_JA,
            RIP_MARKER_REFERENCE_EN,
        )?;
        Ok(ReferenceLabelsTranslation {
            hit,
            rip,
            translation_status: self.translation_status.clone(),
        })
    }

    fn validate_common(&self, source: &Rom) -> Result<()> {
        source.verify_supported_japanese()?;
        ensure!(
            self.format_version == FORMAT_VERSION,
            "graphics translation format_version must be {FORMAT_VERSION}"
        );
        ensure!(
            self.source_rom_sha1 == EXPECTED_SOURCE_SHA1,
            "graphics translation source_rom_sha1 must match the supported Japanese ROM"
        );
        ensure!(
            self.translation_status == COMPLETE_STATUS,
            "graphics translation_status must be {COMPLETE_STATUS} after project-owner approval"
        );
        validate_review(&self.translation_review)?;
        ensure!(
            self.entries.len() == 3,
            "graphics translation must contain exactly three verified assets"
        );
        for expected in [
            ROULETTE_LABEL_ASSET_ID,
            HIT_LABEL_ASSET_ID,
            RIP_MARKER_ASSET_ID,
        ] {
            ensure!(
                self.entries
                    .iter()
                    .filter(|entry| entry.asset_id == expected)
                    .count()
                    == 1,
                "graphics translation must contain exactly one {expected} entry"
            );
        }
        Ok(())
    }

    fn entry(&self, asset_id: &str) -> Result<&GraphicsTranslationEntry> {
        self.entries
            .iter()
            .find(|entry| entry.asset_id == asset_id)
            .ok_or_else(|| anyhow::anyhow!("missing graphics translation asset {asset_id}"))
    }

    fn validate_reference_label(
        &self,
        asset_id: &str,
        source_ja: &str,
        reference_en: &str,
    ) -> Result<ReferenceLabelTranslation> {
        let entry = self.entry(asset_id)?;
        ensure!(
            entry.source_reading_status == STATIC_TILE_LAYOUT_STATUS,
            "{asset_id} source reading must use {STATIC_TILE_LAYOUT_STATUS}"
        );
        ensure!(
            entry.source_ja == source_ja,
            "{asset_id} Japanese source must be {source_ja}"
        );
        ensure!(
            entry.reference_en == reference_en,
            "{asset_id} English reference must be {reference_en}"
        );
        ensure!(
            entry.target_text == reference_en,
            "{asset_id} target text must preserve approved English {reference_en}"
        );
        ensure!(
            entry.target_language == "en",
            "{asset_id} target_language must be en"
        );
        ensure!(
            entry.render_strategy == REFERENCE_PIXEL_RENDER_STRATEGY,
            "{asset_id} render_strategy must be {REFERENCE_PIXEL_RENDER_STRATEGY}"
        );
        ensure!(
            entry.review_status == COMPLETE_STATUS,
            "{asset_id} review_status must be {COMPLETE_STATUS}"
        );
        Ok(ReferenceLabelTranslation {
            asset_id: entry.asset_id.clone(),
            source_ja: entry.source_ja.clone(),
            reference_en: entry.reference_en.clone(),
            text: entry.target_text.clone(),
        })
    }
}

fn validate_roulette_label_text(text: &str) -> Result<()> {
    ensure!(
        !text.is_empty() && text.trim() == text,
        "roulette-label Korean translation must be non-empty without outer whitespace"
    );
    let glyph_count = text.chars().count();
    ensure!(
        glyph_count <= 2,
        "roulette-label Korean translation uses {glyph_count} glyphs; 2 fit the proven sprite budget"
    );
    Ok(())
}

fn validate_review(review: &TranslationReview) -> Result<()> {
    ensure!(
        review.decision == "approved",
        "graphics translation must be approved"
    );
    ensure!(
        review.approved_by == "project_owner",
        "graphics translation must be approved by project_owner"
    );
    ensure!(
        review.approved_on.len() == 10
            && review.approved_on.as_bytes()[4] == b'-'
            && review.approved_on.as_bytes()[7] == b'-',
        "graphics translation approval date must use YYYY-MM-DD"
    );
    ensure!(
        review.scope == "baked_graphics_roulette_hit_rip",
        "graphics translation approval scope is unsupported"
    );
    ensure!(
        review.basis
            == "project_owner_selected_nexon_lv2_gothic_23x12_threshold96_and_contiguous_oam",
        "graphics translation approval basis is unsupported"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_more_than_two_korean_glyphs() {
        let error = validate_roulette_label_text("룰렛모드").unwrap_err();
        assert!(error.to_string().contains("2 fit the proven sprite budget"));
        validate_roulette_label_text("룰렛").unwrap();
    }
}
