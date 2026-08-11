use anyhow::{Result, ensure};
use fontdue::{Font, FontSettings};

use crate::{
    graphics_translation::{GraphicsTranslation, RouletteLabelTranslation},
    rom::{EXPANDED_PRG_SIZE, HEADER_SIZE, Rom},
    tracked::{TrackedImage, WriteReport},
};

use super::{
    build_expanded,
    roulette_render::{CHR_LEN, ExactRender, RenderAttempt, render},
};

const TILE_SIZE: usize = 16;
const ROULETTE_METASPRITE_PRG_OFFSET: usize = 0x090A1;
const ROULETTE_METASPRITE_SOURCE: [u8; 19] = [
    0x06, 0xE3, 0x05, 0x60, 0xED, 0x05, 0x62, 0xF7, 0x05, 0x64, 0x01, 0x05, 0x66, 0x0B, 0x05, 0x68,
    0x15, 0x05, 0x6A,
];
const ROULETTE_METASPRITE_TARGET: [u8; 19] = [
    0x06, 0xE7, 0x05, 0x60, 0xEF, 0x05, 0x62, 0xF7, 0x05, 0x64, 0xFF, 0x05, 0x66, 0x07, 0x05, 0x68,
    0x11, 0x05, 0x6A,
];
const SPRITE_SPACING: usize = 8;
const ROULETTE_LABEL_CHR_OFFSET: usize = 0x01200;
const ROULETTE_LABEL_CHR_LEN: usize = CHR_LEN;
const NEXON_LV2_GOTHIC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/NEXONLv2Gothic.ttf"
));
const FONT_NAME: &str = "NEXON Lv2 Gothic";
const FONT_SIZE: f32 = 13.0;
const THRESHOLD: u8 = 96;
const LETTER_SPACING: usize = 3;
const SHADE_SPLIT: u8 = 60;
const EXPECTED_BODY_WIDTH: usize = 23;
const EXPECTED_BODY_HEIGHT: usize = 12;

#[derive(Debug, Clone)]
pub struct RouletteLabelReport {
    pub source_ja: String,
    pub reference_en: String,
    pub text: String,
    pub translation_status: String,
    pub tile_count: usize,
    pub font: String,
    pub font_size: f32,
    pub threshold: u8,
    pub body_width: usize,
    pub body_height: usize,
    pub sprite_spacing: usize,
}

#[derive(Debug, Clone)]
pub struct RouletteLabelBuild {
    pub data: Vec<u8>,
    pub report: RouletteLabelReport,
    pub writes: Vec<WriteReport>,
}

pub fn build(source: &Rom, translation: &GraphicsTranslation) -> Result<RouletteLabelBuild> {
    source.verify_supported_japanese()?;
    let plan = translation.validate_roulette_label(source)?;
    let rendered = render_roulette_label(&plan)?;
    let baseline = build_expanded(source)?;
    let mut image = TrackedImage::new(baseline.clone());
    image.write_expect(
        "compress ROULETTE metasprite columns to contiguous 8px spacing",
        HEADER_SIZE + ROULETTE_METASPRITE_PRG_OFFSET,
        &ROULETTE_METASPRITE_SOURCE,
        &ROULETTE_METASPRITE_TARGET,
    )?;
    let output_offset = HEADER_SIZE + EXPANDED_PRG_SIZE + ROULETTE_LABEL_CHR_OFFSET;
    image.write_expect(
        "replace baked ROULETTE label CHR tiles 120-129",
        output_offset,
        &source.chr()
            [ROULETTE_LABEL_CHR_OFFSET..ROULETTE_LABEL_CHR_OFFSET + ROULETTE_LABEL_CHR_LEN],
        &rendered.chr,
    )?;
    image.check_untracked_writes(&baseline)?;
    let writes = image.reports().to_vec();

    Ok(RouletteLabelBuild {
        data: image.into_data(),
        report: RouletteLabelReport {
            source_ja: plan.source_ja,
            reference_en: plan.reference_en,
            text: plan.text,
            translation_status: plan.translation_status,
            tile_count: ROULETTE_LABEL_CHR_LEN / TILE_SIZE,
            font: FONT_NAME.to_owned(),
            font_size: FONT_SIZE,
            threshold: THRESHOLD,
            body_width: rendered.body_width,
            body_height: rendered.body_height,
            sprite_spacing: SPRITE_SPACING,
        },
        writes,
    })
}

fn render_roulette_label(plan: &RouletteLabelTranslation) -> Result<ExactRender> {
    let font = Font::from_bytes(NEXON_LV2_GOTHIC, FontSettings::default())
        .map_err(|error| anyhow::anyhow!("load {FONT_NAME}: {error}"))?;
    let rendered = render(
        &font,
        &plan.text,
        FONT_SIZE,
        THRESHOLD,
        LETTER_SPACING,
        SHADE_SPLIT,
    )?;
    let rendered = match rendered {
        RenderAttempt::Fit(rendered) => *rendered,
        RenderAttempt::Rejected { reason, .. } => {
            anyhow::bail!("selected roulette-label renderer rejected text: {reason}")
        }
    };
    ensure!(
        rendered.chr.len() == ROULETTE_LABEL_CHR_LEN,
        "roulette-label CHR length invariant failed"
    );
    ensure!(
        rendered.body_width == EXPECTED_BODY_WIDTH && rendered.body_height == EXPECTED_BODY_HEIGHT,
        "roulette-label selected glyph bounds changed: expected {EXPECTED_BODY_WIDTH}x{EXPECTED_BODY_HEIGHT}, got {}x{}",
        rendered.body_width,
        rendered.body_height
    );
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::SOURCE_CHR_SIZE;
    use sha1::{Digest, Sha1};

    fn test_plan() -> RouletteLabelTranslation {
        RouletteLabelTranslation {
            source_ja: "ルーレット".to_owned(),
            reference_en: "ROULETTE".to_owned(),
            text: "룰렛".to_owned(),
            translation_status: "complete".to_owned(),
        }
    }

    #[test]
    fn compiles_selected_nexon_glyphs_in_five_sprite_columns() {
        let rendered = render_roulette_label(&test_plan()).unwrap();
        let output = rendered.chr;
        assert_eq!(output.len(), 10 * TILE_SIZE);
        assert_eq!((rendered.body_width, rendered.body_height), (23, 12));
        assert_eq!(
            format!("{:x}", Sha1::digest(&output)),
            "7c40aed5958d749834745c96565a2cf8fc11f8a4"
        );
        assert!(output.iter().any(|byte| *byte != 0));
        assert!(
            output
                .chunks_exact(TILE_SIZE)
                .any(|tile| tile[..8] != tile[8..])
        );
        let mut palette_counts = [0_usize; 4];
        for tile in output.chunks_exact(TILE_SIZE) {
            for row in 0..8 {
                for column in 0..8 {
                    let bit = 7 - column;
                    let value = ((tile[row] >> bit) & 1) | (((tile[row + 8] >> bit) & 1) << 1);
                    palette_counts[value as usize] += 1;
                }
            }
        }
        assert!(palette_counts[1..].iter().all(|count| *count > 0));
        assert!(
            output
                .chunks_exact(2 * TILE_SIZE)
                .filter(|tiles| tiles.iter().any(|byte| *byte != 0))
                .count()
                >= 3
        );
    }

    #[test]
    fn preserves_chr_outside_the_ten_owned_tiles() {
        let mut source = vec![0xA5; SOURCE_CHR_SIZE];
        let replacement = render_roulette_label(&test_plan()).unwrap().chr;
        source[ROULETTE_LABEL_CHR_OFFSET..ROULETTE_LABEL_CHR_OFFSET + ROULETTE_LABEL_CHR_LEN]
            .copy_from_slice(&replacement);

        assert_eq!(source[ROULETTE_LABEL_CHR_OFFSET - 1], 0xA5);
        assert_eq!(
            source[ROULETTE_LABEL_CHR_OFFSET + ROULETTE_LABEL_CHR_LEN],
            0xA5
        );
    }

    #[test]
    fn compresses_all_six_metasprite_x_offsets_around_the_same_center() {
        assert_eq!(ROULETTE_METASPRITE_SOURCE[0], 6);
        assert_eq!(ROULETTE_METASPRITE_TARGET[0], 6);
        let source_x = [-29_i8, -19, -9, 1, 11, 21];
        let target_x = [-25_i8, -17, -9, -1, 7, 17];
        for (index, (&source, &target)) in source_x.iter().zip(&target_x).enumerate() {
            let offset = 1 + index * 3;
            assert_eq!(ROULETTE_METASPRITE_SOURCE[offset] as i8, source);
            assert_eq!(ROULETTE_METASPRITE_TARGET[offset] as i8, target);
            assert_eq!(
                &ROULETTE_METASPRITE_TARGET[offset + 1..offset + 3],
                &ROULETTE_METASPRITE_SOURCE[offset + 1..offset + 3]
            );
        }
        assert!(target_x[..5].windows(2).all(|pair| pair[1] - pair[0] == 8));
        assert_eq!(source_x[0] + source_x[5], -8);
        assert_eq!(target_x[0] + target_x[5], -8);
    }
}
