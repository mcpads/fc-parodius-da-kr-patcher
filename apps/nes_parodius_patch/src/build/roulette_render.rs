use anyhow::{Result, ensure};
use fontdue::Font;

pub const LABEL_WIDTH: usize = 40;
pub const LABEL_HEIGHT: usize = 16;
pub const TILE_WIDTH: usize = 8;
pub const TILE_BYTES: usize = 16;
pub const TILE_COLUMNS: usize = 5;
pub const TILE_ROWS: usize = 2;
pub const CHR_LEN: usize = TILE_COLUMNS * TILE_ROWS * TILE_BYTES;

const PALETTE_TRANSPARENT: u8 = 0;
const PALETTE_FILL: u8 = 1;
const PALETTE_SHADE: u8 = 2;
const PALETTE_OUTLINE: u8 = 3;

#[derive(Debug, Clone)]
pub struct ExactRender {
    pub pixels: [u8; LABEL_WIDTH * LABEL_HEIGHT],
    pub chr: Vec<u8>,
    pub body_width: usize,
    pub body_height: usize,
}

#[derive(Debug, Clone)]
pub enum RenderAttempt {
    Fit(Box<ExactRender>),
    Rejected {
        body_width: usize,
        body_height: usize,
        reason: String,
    },
}

struct GlyphBitmap {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
    baseline_top: i32,
}

pub fn render(
    font: &Font,
    text: &str,
    size: f32,
    threshold: u8,
    letter_spacing: usize,
    shade_split: u8,
) -> Result<RenderAttempt> {
    ensure!(!text.is_empty(), "roulette label text must not be empty");
    ensure!(
        size.is_finite() && size > 0.0,
        "roulette label size must be finite and greater than zero"
    );
    ensure!(
        (1..=99).contains(&shade_split),
        "shade split must be between 1 and 99"
    );

    let mut glyphs = Vec::new();
    let mut min_top = i32::MAX;
    let mut max_bottom = i32::MIN;

    for character in text.chars() {
        if font.lookup_glyph_index(character) == 0 {
            return Ok(RenderAttempt::Rejected {
                body_width: 0,
                body_height: 0,
                reason: format!("missing glyph U+{:04X}", character as u32),
            });
        }
        let (metrics, raster) = font.rasterize(character, size);
        let Some((left, top, right, bottom)) =
            visible_bounds(&raster, metrics.width, metrics.height, threshold)
        else {
            return Ok(RenderAttempt::Rejected {
                body_width: 0,
                body_height: 0,
                reason: format!("empty glyph U+{:04X}", character as u32),
            });
        };
        let width = right - left + 1;
        let height = bottom - top + 1;
        let mut pixels = vec![0_u8; width * height];
        for y in 0..height {
            for x in 0..width {
                pixels[y * width + x] = raster[(top + y) * metrics.width + left + x];
            }
        }
        let baseline_top = -metrics.ymin + top as i32;
        min_top = min_top.min(baseline_top);
        max_bottom = max_bottom.max(baseline_top + height as i32);
        glyphs.push(GlyphBitmap {
            pixels,
            width,
            height,
            baseline_top,
        });
    }

    let body_width = glyphs.iter().map(|glyph| glyph.width).sum::<usize>()
        + letter_spacing * glyphs.len().saturating_sub(1);
    let body_height = (max_bottom - min_top) as usize;
    if body_width + 2 > LABEL_WIDTH || body_height + 2 > LABEL_HEIGHT {
        return Ok(RenderAttempt::Rejected {
            body_width,
            body_height,
            reason: "1px outline exceeds 40x16".to_owned(),
        });
    }

    let origin_x = (LABEL_WIDTH - body_width) / 2;
    let origin_y = (LABEL_HEIGHT - body_height) / 2;
    let mut mask = [false; LABEL_WIDTH * LABEL_HEIGHT];
    let mut cursor_x = origin_x;
    for glyph in &glyphs {
        let glyph_y = origin_y as i32 + glyph.baseline_top - min_top;
        for y in 0..glyph.height {
            for x in 0..glyph.width {
                if glyph.pixels[y * glyph.width + x] >= threshold {
                    let output_y = glyph_y as usize + y;
                    let output_x = cursor_x + x;
                    mask[output_y * LABEL_WIDTH + output_x] = true;
                }
            }
        }
        cursor_x += glyph.width + letter_spacing;
    }

    let pixels = compose_tritone(&mask, origin_y, body_height, shade_split);
    let chr = encode_chr(&pixels);
    ensure!(chr.len() == CHR_LEN, "roulette label CHR length mismatch");
    ensure!(
        decode_chr(&chr)? == pixels,
        "roulette label CHR roundtrip changed palette pixels"
    );

    Ok(RenderAttempt::Fit(Box::new(ExactRender {
        pixels,
        chr,
        body_width,
        body_height,
    })))
}

fn visible_bounds(
    raster: &[u8],
    width: usize,
    height: usize,
    threshold: u8,
) -> Option<(usize, usize, usize, usize)> {
    let mut left = width;
    let mut top = height;
    let mut right = 0;
    let mut bottom = 0;
    let mut found = false;
    for y in 0..height {
        for x in 0..width {
            if raster[y * width + x] >= threshold {
                found = true;
                left = left.min(x);
                top = top.min(y);
                right = right.max(x);
                bottom = bottom.max(y);
            }
        }
    }
    found.then_some((left, top, right, bottom))
}

fn compose_tritone(
    mask: &[bool; LABEL_WIDTH * LABEL_HEIGHT],
    body_top: usize,
    body_height: usize,
    shade_split: u8,
) -> [u8; LABEL_WIDTH * LABEL_HEIGHT] {
    let mut canvas = [PALETTE_TRANSPARENT; LABEL_WIDTH * LABEL_HEIGHT];
    for y in 0..LABEL_HEIGHT {
        for x in 0..LABEL_WIDTH {
            if mask[y * LABEL_WIDTH + x] {
                continue;
            }
            let x_start = x.saturating_sub(1);
            let x_end = (x + 1).min(LABEL_WIDTH - 1);
            let y_start = y.saturating_sub(1);
            let y_end = (y + 1).min(LABEL_HEIGHT - 1);
            if (y_start..=y_end).any(|neighbor_y| {
                (x_start..=x_end).any(|neighbor_x| mask[neighbor_y * LABEL_WIDTH + neighbor_x])
            }) {
                canvas[y * LABEL_WIDTH + x] = PALETTE_OUTLINE;
            }
        }
    }

    let shade_start = body_top + body_height * shade_split as usize / 100;
    for y in 0..LABEL_HEIGHT {
        for x in 0..LABEL_WIDTH {
            if mask[y * LABEL_WIDTH + x] {
                canvas[y * LABEL_WIDTH + x] = if y >= shade_start {
                    PALETTE_SHADE
                } else {
                    PALETTE_FILL
                };
            }
        }
    }
    canvas
}

pub fn encode_chr(canvas: &[u8; LABEL_WIDTH * LABEL_HEIGHT]) -> Vec<u8> {
    let mut output = Vec::with_capacity(CHR_LEN);
    for tile_column in 0..TILE_COLUMNS {
        for tile_row in 0..TILE_ROWS {
            let mut plane_0 = [0_u8; TILE_WIDTH];
            let mut plane_1 = [0_u8; TILE_WIDTH];
            for row in 0..TILE_WIDTH {
                for column in 0..TILE_WIDTH {
                    let x = tile_column * TILE_WIDTH + column;
                    let y = tile_row * TILE_WIDTH + row;
                    let value = canvas[y * LABEL_WIDTH + x];
                    plane_0[row] |= (value & 1) << (7 - column);
                    plane_1[row] |= ((value >> 1) & 1) << (7 - column);
                }
            }
            output.extend_from_slice(&plane_0);
            output.extend_from_slice(&plane_1);
        }
    }
    output
}

pub fn decode_chr(chr: &[u8]) -> Result<[u8; LABEL_WIDTH * LABEL_HEIGHT]> {
    ensure!(chr.len() == CHR_LEN, "roulette label CHR must be 160 bytes");
    let mut canvas = [0_u8; LABEL_WIDTH * LABEL_HEIGHT];
    for tile_column in 0..TILE_COLUMNS {
        for tile_row in 0..TILE_ROWS {
            let tile_index = tile_column * TILE_ROWS + tile_row;
            let tile = &chr[tile_index * TILE_BYTES..(tile_index + 1) * TILE_BYTES];
            for row in 0..TILE_WIDTH {
                for column in 0..TILE_WIDTH {
                    let bit = 7 - column;
                    let value =
                        ((tile[row] >> bit) & 1) | (((tile[row + TILE_WIDTH] >> bit) & 1) << 1);
                    let x = tile_column * TILE_WIDTH + column;
                    let y = tile_row * TILE_WIDTH + row;
                    canvas[y * LABEL_WIDTH + x] = value;
                }
            }
        }
    }
    Ok(canvas)
}
