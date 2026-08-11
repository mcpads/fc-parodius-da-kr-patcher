use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{Context, Result, bail, ensure};
use fontdue::{Font, FontSettings};
use serde::Deserialize;

const DALMOORI_TTF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/dalmoori.ttf"
));
const DALMOORI_PIXEL_SIZE: f32 = 8.0;
const COVERAGE_THRESHOLD: u8 = 128;

#[derive(Debug, Clone, Deserialize)]
pub struct PocSpec {
    pub format_version: u8,
    pub target_normalized_index: u8,
    pub target_record: usize,
    pub blank_records: Vec<usize>,
    pub text: String,
    pub tokens: Vec<u8>,
    pub glyphs: Vec<GlyphSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GlyphSpec {
    pub code: u8,
    pub character: char,
}

#[derive(Debug, Clone)]
pub struct ValidatedGlyphs {
    pub tiles: BTreeMap<u8, [u8; 16]>,
}

impl PocSpec {
    pub fn from_path(path: &Path) -> Result<Self> {
        let data = fs::read(path).with_context(|| format!("read PoC spec {}", path.display()))?;
        let spec: Self = serde_json::from_slice(&data)
            .with_context(|| format!("parse PoC spec {}", path.display()))?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn validate(&self) -> Result<ValidatedGlyphs> {
        ensure!(self.format_version == 2, "format_version must be 2");
        ensure!(!self.glyphs.is_empty(), "glyphs must not be empty");
        ensure!(!self.tokens.is_empty(), "tokens must not be empty");
        let mut blank_records = BTreeSet::new();
        for record in &self.blank_records {
            ensure!(
                *record != self.target_record,
                "target_record must not also be blanked"
            );
            ensure!(
                blank_records.insert(*record),
                "duplicate blank record {record}"
            );
        }

        let font = Font::from_bytes(DALMOORI_TTF, FontSettings::default())
            .map_err(|error| anyhow::anyhow!("load vendored Dalmoori font: {error}"))?;
        let mut tiles = BTreeMap::new();
        let mut characters = BTreeMap::new();
        for glyph in &self.glyphs {
            ensure!(
                (1..=0xFD).contains(&glyph.code),
                "glyph code must be between 01 and FD"
            );
            ensure!(
                tiles
                    .insert(glyph.code, rasterize_dalmoori(&font, glyph.character)?)
                    .is_none(),
                "duplicate glyph code {:02X}",
                glyph.code
            );
            characters.insert(glyph.code, glyph.character);
        }
        for token in &self.tokens {
            ensure!(
                tiles.contains_key(token),
                "PoC token {token:02X} has no glyph in the spec"
            );
        }
        let rendered: String = self.tokens.iter().map(|token| characters[token]).collect();
        ensure!(
            rendered == self.text,
            "text does not match the token glyph sequence"
        );
        Ok(ValidatedGlyphs { tiles })
    }
}

pub(crate) fn rasterize_dalmoori(font: &Font, character: char) -> Result<[u8; 16]> {
    ensure!(
        font.lookup_glyph_index(character) != 0,
        "Dalmoori has no glyph for {character:?}"
    );
    let (metrics, bitmap) = font.rasterize(character, DALMOORI_PIXEL_SIZE);
    ensure!(
        metrics.width <= 8 && metrics.height <= 8,
        "Dalmoori glyph {character:?} is {}x{} at 8px",
        metrics.width,
        metrics.height
    );

    let mut tile = [0_u8; 16];
    let offset_x = (8 - metrics.width) / 2;
    // Dalmoori's em square spans y=-1..7 at 8 px. Keep its baseline instead of
    // vertically centering each tight bitmap: punctuation such as '.' and ','
    // otherwise floats in the middle of an 8x8 text cell.
    let offset_y = 7 - metrics.ymin - metrics.height as i32;
    ensure!(
        offset_y >= 0 && offset_y as usize + metrics.height <= 8,
        "Dalmoori glyph {character:?} falls outside the 8x8 baseline cell"
    );
    let offset_y = offset_y as usize;
    for row in 0..metrics.height {
        for column in 0..metrics.width {
            if bitmap[row * metrics.width + column] >= COVERAGE_THRESHOLD {
                tile[offset_y + row] |= 1 << (7 - (offset_x + column));
            }
        }
    }
    ensure!(
        tile[..8].iter().any(|row| *row != 0),
        "Dalmoori glyph {character:?} rasterized to an empty tile"
    );
    Ok(tile)
}

pub(crate) fn rasterize_dalmoori_text(text: &str) -> Result<Vec<[u8; 16]>> {
    let font = Font::from_bytes(DALMOORI_TTF, FontSettings::default())
        .map_err(|error| anyhow::anyhow!("load vendored Dalmoori font: {error}"))?;
    text.chars()
        .map(|character| rasterize_dalmoori(&font, character))
        .collect()
}

pub fn write_preview(path: &Path, spec: &PocSpec, scale: u32) -> Result<()> {
    if scale == 0 {
        bail!("preview scale must be greater than zero");
    }
    let glyphs = spec.validate()?;
    let width = spec.tokens.len() as u32 * 8 * scale;
    let height = 8 * scale;
    let mut pixels = vec![0_u8; (width * height) as usize];
    for (glyph_index, token) in spec.tokens.iter().enumerate() {
        let tile = glyphs.tiles[token];
        for (row, row_bits) in tile.iter().take(8).enumerate() {
            for column in 0..8_usize {
                let on = row_bits & (1 << (7 - column)) != 0;
                for sy in 0..scale as usize {
                    for sx in 0..scale as usize {
                        let x = glyph_index * 8 * scale as usize + column * scale as usize + sx;
                        let y = row * scale as usize + sy;
                        pixels[y * width as usize + x] = if on { 0xFF } else { 0x08 };
                    }
                }
            }
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create preview directory {}", parent.display()))?;
    }
    let file =
        fs::File::create(path).with_context(|| format!("create preview PNG {}", path.display()))?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().context("write preview PNG header")?;
    writer
        .write_image_data(&pixels)
        .context("write preview PNG pixels")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizes_dalmoori_and_validates_text_sequence() {
        let spec = PocSpec {
            format_version: 2,
            target_normalized_index: 0x42,
            target_record: 2,
            blank_records: vec![0, 1],
            text: "파로".to_owned(),
            tokens: vec![1, 2],
            glyphs: vec![
                GlyphSpec {
                    code: 1,
                    character: '파',
                },
                GlyphSpec {
                    code: 2,
                    character: '로',
                },
            ],
        };
        let glyphs = spec.validate().unwrap();
        assert_eq!(
            glyphs.tiles[&1],
            [
                0xFA, 0x52, 0x52, 0x53, 0x52, 0xFA, 0x02, 0x02, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        assert_eq!(
            glyphs.tiles[&2],
            [
                0x7C, 0x0C, 0x70, 0x7C, 0x10, 0x10, 0xFE, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        assert!(glyphs.tiles[&1][..8].iter().any(|row| *row != 0));
        assert!(glyphs.tiles[&2][..8].iter().any(|row| *row != 0));
        assert!(
            glyphs
                .tiles
                .values()
                .all(|tile| tile[8..].iter().all(|byte| *byte == 0))
        );
    }

    #[test]
    fn preserves_dalmoori_baseline_for_sentence_punctuation() {
        let font = Font::from_bytes(DALMOORI_TTF, FontSettings::default()).unwrap();
        let period = rasterize_dalmoori(&font, '.').unwrap();
        let comma = rasterize_dalmoori(&font, ',').unwrap();

        assert!(period[..6].iter().all(|row| *row == 0));
        assert_ne!(period[6], 0);
        assert_eq!(period[7], 0);
        assert!(comma[..6].iter().all(|row| *row == 0));
        assert_ne!(comma[6], 0);
        assert_ne!(comma[7], 0);
    }

    #[test]
    fn rejects_legacy_inline_bitmap_specs() {
        let spec: PocSpec = serde_json::from_str(
            r#"{"format_version":1,"target_normalized_index":66,"target_record":2,"blank_records":[],"text":"파","tokens":[1],"glyphs":[{"code":1,"character":"파"}]}"#,
        )
        .unwrap();
        assert!(
            spec.validate()
                .unwrap_err()
                .to_string()
                .contains("format_version must be 2")
        );
    }
}
