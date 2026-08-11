use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use fontdue::{Font, FontSettings};
use sha1::{Digest, Sha1};

#[cfg(test)]
use super::roulette_render::{CHR_LEN, decode_chr, encode_chr};
use super::roulette_render::{LABEL_HEIGHT, LABEL_WIDTH, RenderAttempt, render};

pub const DEFAULT_SIZES: [f32; 7] = [10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0];

// Approximate RGB presentation of the live sprite palette $0F,$30,$28,$06.
// Candidate validity is based on the palette indices and CHR bytes, not RGB.
const PREVIEW_PALETTE: [[u8; 3]; 4] = [
    [0x08, 0x08, 0x08],
    [0xEC, 0xEE, 0xEC],
    [0xF2, 0xB2, 0x00],
    [0x54, 0x04, 0x00],
];

const PREVIEW_SCALE: usize = 4;
const ROW_LABEL_WIDTH: usize = 144;
const HEADER_HEIGHT: usize = 22;
const CELL_WIDTH: usize = LABEL_WIDTH * PREVIEW_SCALE + 12;
const CELL_HEIGHT: usize = LABEL_HEIGHT * PREVIEW_SCALE + 25;

#[derive(Debug, Clone)]
pub struct SweepReport {
    pub text: String,
    pub width: usize,
    pub height: usize,
    pub threshold: u8,
    pub letter_spacing: usize,
    pub shade_split: u8,
    pub candidates: Vec<CandidateReport>,
}

#[derive(Debug, Clone)]
pub struct CandidateReport {
    pub font: String,
    pub size: f32,
    pub fit: bool,
    pub body_width: usize,
    pub body_height: usize,
    pub chr_sha1: Option<String>,
    pub reason: Option<String>,
}

struct LoadedFont {
    name: String,
    font: Font,
}

struct RenderedCandidate {
    report: CandidateReport,
    pixels: Option<[u8; LABEL_WIDTH * LABEL_HEIGHT]>,
}

pub fn write_sweep(
    output: &Path,
    text: &str,
    font_paths: &[PathBuf],
    sizes: &[f32],
    threshold: u8,
    letter_spacing: usize,
    shade_split: u8,
) -> Result<SweepReport> {
    ensure!(!text.is_empty(), "roulette sweep text must not be empty");
    ensure!(
        !font_paths.is_empty(),
        "roulette sweep requires at least one font"
    );
    ensure!(
        !sizes.is_empty(),
        "roulette sweep requires at least one size"
    );
    ensure!(
        sizes.iter().all(|size| size.is_finite() && *size > 0.0),
        "roulette sweep sizes must be finite and greater than zero"
    );
    ensure!(
        (1..=99).contains(&shade_split),
        "shade split must be between 1 and 99"
    );

    let fonts = load_fonts(font_paths)?;
    let mut rendered = Vec::with_capacity(fonts.len() * sizes.len());
    for loaded in &fonts {
        for &size in sizes {
            rendered.push(render_candidate(
                loaded,
                text,
                size,
                threshold,
                letter_spacing,
                shade_split,
            )?);
        }
    }

    let pixels = render_contact_sheet(&fonts, sizes, &rendered);
    let sheet_width = ROW_LABEL_WIDTH + sizes.len() * CELL_WIDTH;
    let sheet_height = HEADER_HEIGHT + fonts.len() * CELL_HEIGHT;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create sweep output directory {}", parent.display()))?;
    }
    let file = fs::File::create(output)
        .with_context(|| format!("create roulette sweep PNG {}", output.display()))?;
    let mut encoder = png::Encoder::new(file, sheet_width as u32, sheet_height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .context("write roulette sweep PNG header")?;
    writer
        .write_image_data(&pixels)
        .context("write roulette sweep PNG pixels")?;

    Ok(SweepReport {
        text: text.to_owned(),
        width: LABEL_WIDTH,
        height: LABEL_HEIGHT,
        threshold,
        letter_spacing,
        shade_split,
        candidates: rendered
            .into_iter()
            .map(|candidate| candidate.report)
            .collect(),
    })
}

fn load_fonts(paths: &[PathBuf]) -> Result<Vec<LoadedFont>> {
    paths
        .iter()
        .map(|path| {
            let data =
                fs::read(path).with_context(|| format!("read sweep font {}", path.display()))?;
            let font = Font::from_bytes(data, FontSettings::default())
                .map_err(|error| anyhow::anyhow!("load sweep font {}: {error}", path.display()))?;
            let name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("font")
                .to_owned();
            Ok(LoadedFont { name, font })
        })
        .collect()
}

fn render_candidate(
    loaded: &LoadedFont,
    text: &str,
    size: f32,
    threshold: u8,
    letter_spacing: usize,
    shade_split: u8,
) -> Result<RenderedCandidate> {
    match render(
        &loaded.font,
        text,
        size,
        threshold,
        letter_spacing,
        shade_split,
    )? {
        RenderAttempt::Fit(exact) => {
            let exact = *exact;
            let chr_sha1 = format!("{:x}", Sha1::digest(&exact.chr));
            Ok(RenderedCandidate {
                report: CandidateReport {
                    font: loaded.name.clone(),
                    size,
                    fit: true,
                    body_width: exact.body_width,
                    body_height: exact.body_height,
                    chr_sha1: Some(chr_sha1),
                    reason: None,
                },
                pixels: Some(exact.pixels),
            })
        }
        RenderAttempt::Rejected {
            body_width,
            body_height,
            reason,
        } => Ok(RenderedCandidate {
            report: CandidateReport {
                font: loaded.name.clone(),
                size,
                fit: false,
                body_width,
                body_height,
                chr_sha1: None,
                reason: Some(reason),
            },
            pixels: None,
        }),
    }
}

fn render_contact_sheet(
    fonts: &[LoadedFont],
    sizes: &[f32],
    candidates: &[RenderedCandidate],
) -> Vec<u8> {
    let width = ROW_LABEL_WIDTH + sizes.len() * CELL_WIDTH;
    let height = HEADER_HEIGHT + fonts.len() * CELL_HEIGHT;
    let mut output = vec![0x18_u8; width * height * 3];

    draw_text(&mut output, width, 8, 7, "FONT / SIZE", [0xC8; 3]);
    for (column, size) in sizes.iter().enumerate() {
        let label = format!("{size:.0}PX");
        draw_text(
            &mut output,
            width,
            ROW_LABEL_WIDTH + column * CELL_WIDTH + 6,
            7,
            &label,
            [0xE8; 3],
        );
    }

    for (row, font) in fonts.iter().enumerate() {
        let row_y = HEADER_HEIGHT + row * CELL_HEIGHT;
        draw_text(&mut output, width, 8, row_y + 8, &font.name, [0xE8; 3]);
        for column in 0..sizes.len() {
            let candidate = &candidates[row * sizes.len() + column];
            let cell_x = ROW_LABEL_WIDTH + column * CELL_WIDTH;
            fill_rect(
                &mut output,
                width,
                cell_x,
                row_y,
                CELL_WIDTH - 2,
                CELL_HEIGHT - 2,
                if candidate.report.fit {
                    [0x28, 0x28, 0x28]
                } else {
                    [0x40, 0x18, 0x18]
                },
            );
            let preview_x = cell_x + 6;
            let preview_y = row_y + 6;
            if let Some(pixels) = &candidate.pixels {
                draw_indexed_preview(
                    &mut output,
                    width,
                    preview_x,
                    preview_y,
                    pixels,
                    PREVIEW_SCALE,
                );
                let bounds = format!(
                    "{}X{}",
                    candidate.report.body_width, candidate.report.body_height
                );
                draw_text(
                    &mut output,
                    width,
                    preview_x,
                    row_y + LABEL_HEIGHT * PREVIEW_SCALE + 11,
                    &bounds,
                    [0xA8; 3],
                );
            } else {
                draw_cross(
                    &mut output,
                    width,
                    preview_x,
                    preview_y,
                    LABEL_WIDTH * PREVIEW_SCALE,
                    LABEL_HEIGHT * PREVIEW_SCALE,
                    [0xE0, 0x38, 0x38],
                );
                let bounds = format!(
                    "NO FIT {}X{}",
                    candidate.report.body_width, candidate.report.body_height
                );
                draw_text(
                    &mut output,
                    width,
                    preview_x,
                    row_y + LABEL_HEIGHT * PREVIEW_SCALE + 11,
                    &bounds,
                    [0xF0, 0x70, 0x70],
                );
            }
        }
    }
    output
}

fn draw_indexed_preview(
    output: &mut [u8],
    output_width: usize,
    origin_x: usize,
    origin_y: usize,
    pixels: &[u8; LABEL_WIDTH * LABEL_HEIGHT],
    scale: usize,
) {
    for y in 0..LABEL_HEIGHT {
        for x in 0..LABEL_WIDTH {
            let color = PREVIEW_PALETTE[pixels[y * LABEL_WIDTH + x] as usize];
            fill_rect(
                output,
                output_width,
                origin_x + x * scale,
                origin_y + y * scale,
                scale,
                scale,
                color,
            );
        }
    }
}

fn draw_cross(
    output: &mut [u8],
    output_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: [u8; 3],
) {
    let diagonal = width.min(height);
    for offset in 0..diagonal {
        set_pixel(output, output_width, x + offset, y + offset, color);
        set_pixel(
            output,
            output_width,
            x + width - 1 - offset,
            y + offset,
            color,
        );
    }
}

fn fill_rect(
    output: &mut [u8],
    output_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: [u8; 3],
) {
    for output_y in y..y + height {
        for output_x in x..x + width {
            set_pixel(output, output_width, output_x, output_y, color);
        }
    }
}

fn set_pixel(output: &mut [u8], output_width: usize, x: usize, y: usize, color: [u8; 3]) {
    let offset = (y * output_width + x) * 3;
    if offset + 3 <= output.len() {
        output[offset..offset + 3].copy_from_slice(&color);
    }
}

fn draw_text(
    output: &mut [u8],
    output_width: usize,
    x: usize,
    y: usize,
    text: &str,
    color: [u8; 3],
) {
    let mut cursor_x = x;
    for character in text.chars().flat_map(char::to_uppercase) {
        let glyph = tiny_glyph(character);
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    set_pixel(output, output_width, cursor_x + column, y + row, color);
                }
            }
        }
        cursor_x += 6;
    }
}

fn tiny_glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0E],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x10, 0x1E, 0x01, 0x01, 0x1E],
        '6' => [0x0E, 0x10, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x01, 0x0E],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '-' => [0, 0, 0, 0x1F, 0, 0, 0],
        '.' => [0, 0, 0, 0, 0, 0x0C, 0x0C],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DALMOORI: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/fonts/dalmoori.ttf"
    ));

    fn test_font() -> LoadedFont {
        LoadedFont {
            name: "dalmoori".to_owned(),
            font: Font::from_bytes(DALMOORI, FontSettings::default()).unwrap(),
        }
    }

    #[test]
    fn exact_chr_roundtrip_preserves_all_palette_indices() {
        let candidate = render_candidate(&test_font(), "룰렛", 12.0, 128, 3, 60).unwrap();
        assert!(candidate.report.fit);
        let pixels = candidate.pixels.unwrap();
        assert!((0..=3).all(|value| pixels.contains(&value)));
        let chr = encode_chr(&pixels);
        assert_eq!(chr.len(), CHR_LEN);
        assert_eq!(decode_chr(&chr).unwrap(), pixels);
    }

    #[test]
    fn rejects_candidates_that_cannot_keep_the_outline_inside_the_surface() {
        let candidate = render_candidate(&test_font(), "룰렛", 64.0, 128, 3, 60).unwrap();
        assert!(!candidate.report.fit);
        assert!(candidate.pixels.is_none());
        assert_eq!(
            candidate.report.reason.as_deref(),
            Some("1px outline exceeds 40x16")
        );
    }

    #[test]
    fn validates_sweep_parameters_before_loading_fonts() {
        let error = write_sweep(
            Path::new("unused.png"),
            "룰렛",
            &[],
            &DEFAULT_SIZES,
            128,
            3,
            60,
        )
        .unwrap_err();
        assert!(error.to_string().contains("requires at least one font"));
    }
}
