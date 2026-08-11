use std::{collections::BTreeMap, fs, io::Cursor, path::Path};

use anyhow::{Context, Result, ensure};

use crate::{
    rom::{EXPANDED_PRG_SIZE, HEADER_SIZE, Rom, SOURCE_CHR_SIZE},
    tracked::{TrackedImage, WriteReport},
};

use super::build_expanded;

const TILE_SIZE: usize = 16;
const TILE_WIDTH: usize = 8;
const TITLE_TILE_COLUMNS: usize = 24;
const TITLE_TILE_ROWS: usize = 4;
const TITLE_WIDTH: usize = TITLE_TILE_COLUMNS * TILE_WIDTH;
const TITLE_HEIGHT: usize = TITLE_TILE_ROWS * TILE_WIDTH;
const TITLE_C06_CHR_OFFSET: usize = 0x0C000;
const TITLE_RANGE_A: std::ops::RangeInclusive<u8> = 0x24..=0x30;
const TITLE_RANGE_B: std::ops::RangeInclusive<u8> = 0x32..=0x6D;
const TITLE_RANGE_C: std::ops::RangeInclusive<u8> = 0x70..=0x75;
const TITLE_RANGE_D: std::ops::RangeInclusive<u8> = 0xA0..=0xA0;
const TITLE_RANGE_E: std::ops::RangeInclusive<u8> = 0xAF..=0xB7;
const TITLE_TILE_CAPACITY: usize = 89;
const ALPHA_THRESHOLD: u8 = 128;
const TITLE_FILL_INDEX: u8 = 1;
const TITLE_HIGHLIGHT_INDEX: u8 = 3;
const SOLID_FILL_HEIGHT_DIVISOR: usize = 5;
const TITLE_GLYPH_X_RANGES: [(usize, usize); 7] = [
    (0, 34),    // 파
    (34, 61),   // 로
    (61, 90),   // 디
    (90, 125),  // 우
    (125, 150), // 스
    (150, 183), // 다
    (183, 192), // !
];

const NES_TITLE_PALETTE: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x00],
    [0xFE, 0x6E, 0xCC],
    [0xEA, 0x9E, 0x22],
    [0xFE, 0xC4, 0xEA],
];

const SPARSE_TITLE_RUNS: &[(usize, &[u8])] = &[
    (0x15F42, &[0x26]),
    (0x15F46, &[0xB6, 0xB7, 0x24]),
    (0x15F4C, &[0x27, 0x35, 0x28, 0x29, 0x26, 0x00, 0x62]),
    (0x15F56, &[0x62, 0x5E, 0x5E, 0x62]),
    (0x15FCB, &[0x63, 0x7B]),
];

const TITLE_ROW_RUNS: [(usize, &[u8]); TITLE_TILE_ROWS] = [
    (
        0x15F5D,
        &[
            0x30, 0x36, 0x32, 0x33, 0x34, 0x35, 0x35, 0x36, 0x37, 0x38, 0x38, 0x39, 0x3A, 0x27,
            0x4D, 0x35, 0x3F, 0xB1, 0xAF, 0xA0, 0xB0, 0xB1, 0x71, 0x72,
        ],
    ),
    (
        0x15F78,
        &[
            0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x25, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C,
            0x5D, 0x4E, 0x4F, 0xB2, 0x41, 0xB3, 0xB4, 0xB5, 0x70, 0x41,
        ],
    ),
    (
        0x15F93,
        &[
            0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x25, 0x57, 0x58, 0x59, 0x5A, 0x5B, 0x5C,
            0x6D, 0x58, 0x5F, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x73, 0x74,
        ],
    ),
    (
        0x15FAE,
        &[
            0x60, 0x61, 0x00, 0x63, 0x64, 0x65, 0x65, 0x66, 0x63, 0x68, 0x69, 0x6A, 0x6B, 0x6C,
            0x65, 0x3B, 0x69, 0x3B, 0x3C, 0x3D, 0x3E, 0x65, 0x3E, 0x75, 0xBD, 0x1D,
        ],
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoundingBox {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[derive(Debug, Clone)]
struct IndexedTitle {
    pixels: Vec<u8>,
    source_width: usize,
    source_height: usize,
    source_bounds: BoundingBox,
}

#[derive(Debug, Clone)]
struct CompiledTiles {
    cells: Vec<u8>,
    tiles: BTreeMap<u8, [u8; TILE_SIZE]>,
}

#[derive(Debug, Clone)]
pub struct TitlePocReport {
    pub source_width: usize,
    pub source_height: usize,
    pub source_bounds: (usize, usize, usize, usize),
    pub tile_count: usize,
    pub tile_capacity: usize,
}

#[derive(Debug, Clone)]
pub struct TitlePocBuild {
    pub data: Vec<u8>,
    pub report: TitlePocReport,
    pub writes: Vec<WriteReport>,
}

pub fn build(source: &Rom, asset_png: &[u8]) -> Result<TitlePocBuild> {
    source.verify_supported_japanese()?;
    let indexed = decode_and_quantize(asset_png)?;
    let compiled = compile_tiles(&indexed)?;
    let baseline = build_expanded(source)?;
    let patched_chr = compile_title_chr(source.chr(), &compiled)?;
    let mut image = TrackedImage::new(baseline.clone());

    patch_title_commands(&mut image, &compiled.cells)?;
    let chr_start = HEADER_SIZE + EXPANDED_PRG_SIZE;
    for (label, tile_range) in [
        ("replace title CHR tiles 24-30", TITLE_RANGE_A),
        ("replace title CHR tiles 32-6D", TITLE_RANGE_B),
        ("replace title CHR tiles 70-75", TITLE_RANGE_C),
        ("replace title CHR tile A0", TITLE_RANGE_D),
        ("replace title CHR tiles AF-B7", TITLE_RANGE_E),
    ] {
        let range = chr_byte_range(tile_range);
        image.write_expect(
            label,
            chr_start + range.start,
            &source.chr()[range.clone()],
            &patched_chr[range],
        )?;
    }
    image.check_untracked_writes(&baseline)?;
    let writes = image.reports().to_vec();
    Ok(TitlePocBuild {
        data: image.into_data(),
        report: TitlePocReport {
            source_width: indexed.source_width,
            source_height: indexed.source_height,
            source_bounds: (
                indexed.source_bounds.x,
                indexed.source_bounds.y,
                indexed.source_bounds.width,
                indexed.source_bounds.height,
            ),
            tile_count: compiled.tiles.len(),
            tile_capacity: TITLE_TILE_CAPACITY,
        },
        writes,
    })
}

pub fn write_preview(path: &Path, asset_png: &[u8], scale: u32) -> Result<()> {
    ensure!(scale > 0, "preview scale must be greater than zero");
    let indexed = decode_and_quantize(asset_png)?;
    let width = TITLE_WIDTH as u32 * scale;
    let height = TITLE_HEIGHT as u32 * scale;
    let mut pixels = vec![0_u8; (width * height * 3) as usize];
    for y in 0..TITLE_HEIGHT {
        for x in 0..TITLE_WIDTH {
            let color = NES_TITLE_PALETTE[indexed.pixels[y * TITLE_WIDTH + x] as usize];
            for sy in 0..scale as usize {
                for sx in 0..scale as usize {
                    let output_x = x * scale as usize + sx;
                    let output_y = y * scale as usize + sy;
                    let offset = (output_y * width as usize + output_x) * 3;
                    pixels[offset..offset + 3].copy_from_slice(&color);
                }
            }
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create preview directory {}", parent.display()))?;
    }
    let file = fs::File::create(path)
        .with_context(|| format!("create title preview PNG {}", path.display()))?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .context("write title preview header")?;
    writer
        .write_image_data(&pixels)
        .context("write title preview pixels")?;
    Ok(())
}

fn decode_and_quantize(asset_png: &[u8]) -> Result<IndexedTitle> {
    let decoder = png::Decoder::new(Cursor::new(asset_png));
    let mut reader = decoder.read_info().context("read title PNG header")?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow::anyhow!("title PNG output buffer is too large"))?;
    let mut buffer = vec![0; size];
    let info = reader
        .next_frame(&mut buffer)
        .context("decode title PNG frame")?;
    ensure!(
        info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight,
        "title PNG must be 8-bit RGBA"
    );
    let width = info.width as usize;
    let height = info.height as usize;
    let rgba = &buffer[..info.buffer_size()];
    let bounds = alpha_bounds(rgba, width, height)?;

    let mut pixels = vec![0_u8; TITLE_WIDTH * TITLE_HEIGHT];
    for y in 0..TITLE_HEIGHT {
        let source_y = bounds.y + sample_axis(y, TITLE_HEIGHT, bounds.height);
        for x in 0..TITLE_WIDTH {
            let source_x = bounds.x + sample_axis(x, TITLE_WIDTH, bounds.width);
            let offset = (source_y * width + source_x) * 4;
            if rgba[offset + 3] >= ALPHA_THRESHOLD {
                pixels[y * TITLE_WIDTH + x] =
                    nearest_palette_index([rgba[offset], rgba[offset + 1], rgba[offset + 2]]);
            }
        }
    }
    apply_checkerboard_highlight(&mut pixels);
    ensure!(
        pixels.iter().any(|pixel| *pixel != 0),
        "title PNG produced no opaque pixels"
    );
    Ok(IndexedTitle {
        pixels,
        source_width: width,
        source_height: height,
        source_bounds: bounds,
    })
}

fn apply_checkerboard_highlight(pixels: &mut [u8]) {
    debug_assert_eq!(pixels.len(), TITLE_WIDTH * TITLE_HEIGHT);
    for (glyph_start_x, glyph_end_x) in TITLE_GLYPH_X_RANGES {
        debug_assert!(glyph_start_x < glyph_end_x && glyph_end_x <= TITLE_WIDTH);
        let mut fill_rows = (0..TITLE_HEIGHT).filter(|y| {
            (glyph_start_x..glyph_end_x).any(|x| pixels[y * TITLE_WIDTH + x] == TITLE_FILL_INDEX)
        });
        let Some(glyph_start_y) = fill_rows.clone().next() else {
            continue;
        };
        let glyph_end_y = fill_rows
            .next_back()
            .expect("glyph fill has at least one occupied row")
            + 1;
        let solid_rows = (glyph_end_y - glyph_start_y).div_ceil(SOLID_FILL_HEIGHT_DIVISOR);
        for y in glyph_start_y + solid_rows..glyph_end_y {
            for x in glyph_start_x..glyph_end_x {
                if pixels[y * TITLE_WIDTH + x] == TITLE_FILL_INDEX && (x + y) % 2 == 0 {
                    pixels[y * TITLE_WIDTH + x] = TITLE_HIGHLIGHT_INDEX;
                }
            }
        }
    }
}

fn alpha_bounds(rgba: &[u8], width: usize, height: usize) -> Result<BoundingBox> {
    ensure!(
        rgba.len() == width * height * 4,
        "RGBA buffer size does not match dimensions"
    );
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;
    for y in 0..height {
        for x in 0..width {
            if rgba[(y * width + x) * 4 + 3] >= ALPHA_THRESHOLD {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                found = true;
            }
        }
    }
    ensure!(found, "title PNG has no opaque content");
    Ok(BoundingBox {
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    })
}

fn sample_axis(output: usize, output_size: usize, source_size: usize) -> usize {
    ((output * 2 + 1) * source_size / (output_size * 2)).min(source_size - 1)
}

fn nearest_palette_index(rgb: [u8; 3]) -> u8 {
    (1_u8..=3)
        .min_by_key(|index| {
            let target = NES_TITLE_PALETTE[*index as usize];
            rgb.into_iter()
                .zip(target)
                .map(|(actual, expected)| {
                    let delta = actual as i32 - expected as i32;
                    delta * delta
                })
                .sum::<i32>()
        })
        .unwrap()
}

fn compile_tiles(indexed: &IndexedTitle) -> Result<CompiledTiles> {
    ensure!(
        indexed.pixels.len() == TITLE_WIDTH * TITLE_HEIGHT,
        "indexed title dimensions are invalid"
    );
    let available_codes = available_tile_codes();
    ensure!(
        available_codes.len() == TITLE_TILE_CAPACITY,
        "title tile capacity invariant failed"
    );
    let mut pattern_indices = BTreeMap::<[u8; TILE_SIZE], usize>::new();
    let mut patterns = Vec::new();
    let mut cell_patterns = Vec::with_capacity(TITLE_TILE_COLUMNS * TITLE_TILE_ROWS);
    for tile_y in 0..TITLE_TILE_ROWS {
        for tile_x in 0..TITLE_TILE_COLUMNS {
            let tile = encode_tile(&indexed.pixels, tile_x, tile_y);
            if tile.iter().all(|byte| *byte == 0) {
                cell_patterns.push(None);
                continue;
            }
            let index = if let Some(index) = pattern_indices.get(&tile) {
                *index
            } else {
                let index = patterns.len();
                patterns.push(tile);
                pattern_indices.insert(tile, index);
                index
            };
            cell_patterns.push(Some(index));
        }
    }
    ensure!(
        patterns.len() <= TITLE_TILE_CAPACITY,
        "title needs {} unique nonblank tiles; capacity is {TITLE_TILE_CAPACITY}",
        patterns.len()
    );
    let mut tiles = BTreeMap::new();
    for (index, tile) in patterns.into_iter().enumerate() {
        tiles.insert(available_codes[index], tile);
    }
    let cells: Vec<u8> = cell_patterns
        .into_iter()
        .map(|index| index.map_or(0, |index| available_codes[index]))
        .collect();
    ensure!(
        cells.len() == TITLE_TILE_COLUMNS * TITLE_TILE_ROWS,
        "title cell count invariant failed"
    );
    Ok(CompiledTiles { cells, tiles })
}

fn encode_tile(pixels: &[u8], tile_x: usize, tile_y: usize) -> [u8; TILE_SIZE] {
    let mut tile = [0_u8; TILE_SIZE];
    for row in 0..TILE_WIDTH {
        for column in 0..TILE_WIDTH {
            let pixel =
                pixels[(tile_y * TILE_WIDTH + row) * TITLE_WIDTH + tile_x * TILE_WIDTH + column];
            let bit = 1 << (7 - column);
            if pixel & 1 != 0 {
                tile[row] |= bit;
            }
            if pixel & 2 != 0 {
                tile[row + 8] |= bit;
            }
        }
    }
    tile
}

fn compile_title_chr(source_chr: &[u8], compiled: &CompiledTiles) -> Result<Vec<u8>> {
    ensure!(
        source_chr.len() == SOURCE_CHR_SIZE,
        "source CHR must be {SOURCE_CHR_SIZE} bytes"
    );
    let mut output = source_chr.to_vec();
    for range in [
        TITLE_RANGE_A,
        TITLE_RANGE_B,
        TITLE_RANGE_C,
        TITLE_RANGE_D,
        TITLE_RANGE_E,
    ] {
        output[chr_byte_range(range)].fill(0);
    }
    for (code, tile) in &compiled.tiles {
        let offset = tile_offset(*code);
        output[offset..offset + TILE_SIZE].copy_from_slice(tile);
    }
    Ok(output)
}

fn patch_title_commands(image: &mut TrackedImage, cells: &[u8]) -> Result<()> {
    ensure!(
        cells.len() == TITLE_TILE_COLUMNS * TITLE_TILE_ROWS,
        "title cell count must be {}",
        TITLE_TILE_COLUMNS * TITLE_TILE_ROWS
    );
    for (index, (offset, expected)) in SPARSE_TITLE_RUNS.iter().enumerate() {
        image.write_expect(
            format!("blank Japanese title sparse run {index}"),
            *offset,
            expected,
            &vec![0; expected.len()],
        )?;
    }
    for (row, (offset, expected)) in TITLE_ROW_RUNS.iter().enumerate() {
        let start = row * TITLE_TILE_COLUMNS;
        let mut replacement = cells[start..start + TITLE_TILE_COLUMNS].to_vec();
        if expected.len() > TITLE_TILE_COLUMNS {
            replacement.resize(expected.len(), 0);
        }
        image.write_expect(
            format!("replace Korean title row {row}"),
            *offset,
            expected,
            &replacement,
        )?;
    }
    Ok(())
}

fn chr_byte_range(tiles: std::ops::RangeInclusive<u8>) -> std::ops::Range<usize> {
    let start = tile_offset(*tiles.start());
    let end = tile_offset(*tiles.end()) + TILE_SIZE;
    start..end
}

fn tile_offset(code: u8) -> usize {
    TITLE_C06_CHR_OFFSET + code as usize * TILE_SIZE
}

fn available_tile_codes() -> Vec<u8> {
    TITLE_RANGE_A
        .chain(TITLE_RANGE_B)
        .chain(TITLE_RANGE_C)
        .chain(TITLE_RANGE_D)
        .chain(TITLE_RANGE_E)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE_ASSET: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/title/korean-title-genai.png"
    ));

    #[test]
    fn converts_committed_title_within_the_proven_tile_budget() {
        let indexed = decode_and_quantize(TITLE_ASSET).unwrap();
        let compiled = compile_tiles(&indexed).unwrap();

        assert_eq!((indexed.source_width, indexed.source_height), (2032, 774));
        assert_eq!(compiled.tiles.len(), 88);
        assert_eq!(compiled.cells.len(), TITLE_TILE_COLUMNS * TITLE_TILE_ROWS);
        assert!(indexed.pixels.contains(&1));
        assert!(indexed.pixels.contains(&2));
        assert!(indexed.pixels.contains(&TITLE_HIGHLIGHT_INDEX));
        assert!(!compiled.tiles.contains_key(&0x31));
    }

    #[test]
    fn glyph_regions_partition_the_title_and_each_contains_fill() {
        let indexed = decode_and_quantize(TITLE_ASSET).unwrap();
        let mut expected_start = 0;

        for (start_x, end_x) in TITLE_GLYPH_X_RANGES {
            assert_eq!(start_x, expected_start);
            assert!(start_x < end_x && end_x <= TITLE_WIDTH);
            assert!((0..TITLE_HEIGHT).any(|y| {
                (start_x..end_x).any(|x| indexed.pixels[y * TITLE_WIDTH + x] == TITLE_FILL_INDEX)
            }));
            expected_start = end_x;
        }

        assert_eq!(expected_start, TITLE_WIDTH);
    }

    #[test]
    fn adds_the_japanese_checkerboard_only_to_the_lower_pink_fill() {
        let mut pixels = vec![TITLE_FILL_INDEX; TITLE_WIDTH * TITLE_HEIGHT];

        apply_checkerboard_highlight(&mut pixels);

        let checkerboard_start_row = TITLE_HEIGHT.div_ceil(SOLID_FILL_HEIGHT_DIVISOR);
        assert!(
            pixels[..checkerboard_start_row * TITLE_WIDTH]
                .iter()
                .all(|pixel| *pixel == TITLE_FILL_INDEX)
        );
        for y in checkerboard_start_row..TITLE_HEIGHT {
            for x in 0..TITLE_WIDTH {
                let expected = if (x + y) % 2 == 0 {
                    TITLE_HIGHLIGHT_INDEX
                } else {
                    TITLE_FILL_INDEX
                };
                assert_eq!(pixels[y * TITLE_WIDTH + x], expected);
            }
        }
    }

    #[test]
    fn measures_one_solid_band_across_disconnected_parts_of_a_glyph() {
        let mut pixels = vec![0; TITLE_WIDTH * TITLE_HEIGHT];
        for y in 10..20 {
            for x in 11..19 {
                pixels[y * TITLE_WIDTH + x] = TITLE_FILL_INDEX;
            }
        }
        for y in 24..29 {
            pixels[y * TITLE_WIDTH + 11] = TITLE_FILL_INDEX;
        }

        apply_checkerboard_highlight(&mut pixels);

        for y in 10..14 {
            assert!(
                pixels[y * TITLE_WIDTH + 11..y * TITLE_WIDTH + 19]
                    .iter()
                    .all(|pixel| *pixel == TITLE_FILL_INDEX)
            );
        }
        for y in 14..20 {
            for x in 11..19 {
                let expected = if (x + y) % 2 == 0 {
                    TITLE_HIGHLIGHT_INDEX
                } else {
                    TITLE_FILL_INDEX
                };
                assert_eq!(pixels[y * TITLE_WIDTH + x], expected);
            }
        }
        for y in 24..29 {
            let expected = if (11 + y) % 2 == 0 {
                TITLE_HIGHLIGHT_INDEX
            } else {
                TITLE_FILL_INDEX
            };
            assert_eq!(pixels[y * TITLE_WIDTH + 11], expected);
        }
    }

    #[test]
    fn preserves_protected_chr_and_clears_unused_title_slots() {
        let indexed = decode_and_quantize(TITLE_ASSET).unwrap();
        let compiled = compile_tiles(&indexed).unwrap();
        let source = vec![0xA5; SOURCE_CHR_SIZE];
        let output = compile_title_chr(&source, &compiled).unwrap();

        assert_eq!(
            &output[tile_offset(0x31)..tile_offset(0x31) + TILE_SIZE],
            &[0xA5; TILE_SIZE]
        );
        assert_eq!(
            &output[tile_offset(0x23)..tile_offset(0x23) + TILE_SIZE],
            &[0xA5; TILE_SIZE]
        );
        assert_eq!(
            &output[tile_offset(0x6E)..tile_offset(0x6E) + TILE_SIZE],
            &[0xA5; TILE_SIZE]
        );
        for code in available_tile_codes() {
            let tile = &output[tile_offset(code)..tile_offset(code) + TILE_SIZE];
            if let Some(expected) = compiled.tiles.get(&code) {
                assert_eq!(tile, expected);
            } else {
                assert_eq!(tile, &[0; TILE_SIZE]);
            }
        }
    }

    #[test]
    fn replaces_only_literal_title_command_runs() {
        let indexed = decode_and_quantize(TITLE_ASSET).unwrap();
        let compiled = compile_tiles(&indexed).unwrap();
        let mut baseline = vec![0_u8; 0x16000];
        for (offset, expected) in SPARSE_TITLE_RUNS {
            baseline[*offset..*offset + expected.len()].copy_from_slice(expected);
        }
        for (offset, expected) in TITLE_ROW_RUNS {
            baseline[offset..offset + expected.len()].copy_from_slice(expected);
        }
        let mut image = TrackedImage::new(baseline.clone());
        patch_title_commands(&mut image, &compiled.cells).unwrap();
        image.check_untracked_writes(&baseline).unwrap();
        let output = image.into_data();

        for (offset, expected) in SPARSE_TITLE_RUNS {
            assert_eq!(
                &output[*offset..*offset + expected.len()],
                &vec![0; expected.len()]
            );
        }
        for (row, (offset, expected)) in TITLE_ROW_RUNS.into_iter().enumerate() {
            let start = row * TITLE_TILE_COLUMNS;
            assert_eq!(
                &output[offset..offset + TITLE_TILE_COLUMNS],
                &compiled.cells[start..start + TITLE_TILE_COLUMNS]
            );
            assert!(
                output[offset + TITLE_TILE_COLUMNS..offset + expected.len()]
                    .iter()
                    .all(|byte| *byte == 0)
            );
        }
    }

    #[test]
    fn rejects_an_empty_alpha_image() {
        let error = alpha_bounds(&[0; 4 * 4 * 4], 4, 4).unwrap_err();
        assert!(error.to_string().contains("no opaque content"));
    }
}
