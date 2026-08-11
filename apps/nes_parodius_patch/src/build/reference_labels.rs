use anyhow::{Result, ensure};

use crate::{
    graphics_translation::{GraphicsTranslation, ReferenceLabelsTranslation},
    rom::{EXPANDED_PRG_SIZE, HEADER_SIZE, Rom},
    sha1_hex,
    tracked::{TrackedImage, WriteReport},
};

use super::build_expanded;

const TILE_SIZE: usize = 16;
const HIT_TARGET_SHA1: &str = "5fbff7514db49e1f68dec8ff43028fda9d8283d7";
const RIP_TARGET_SHA1: &str = "82e7eab97c24d3ba46f8d0c9014a5fb614286a80";

const HIT_TILE_OFFSETS: [usize; 4] = [0x15DB0, 0x15DD0, 0x15DE0, 0x15E00];
const HIT_XOR_MASKS: [[u8; TILE_SIZE]; 4] = [
    [
        0x00, 0x03, 0x01, 0x00, 0x07, 0x00, 0x07, 0x00, 0x00, 0x03, 0x03, 0x07, 0x07, 0x0D, 0x0D,
        0x0D,
    ],
    [
        0x00, 0x60, 0x40, 0x30, 0xD0, 0x10, 0xD0, 0x10, 0x00, 0xE0, 0xF0, 0xD0, 0xF0, 0x48, 0x60,
        0x60,
    ],
    [
        0x07, 0x00, 0x01, 0x01, 0x03, 0x00, 0x00, 0x00, 0x09, 0x0A, 0x09, 0x09, 0x03, 0x00, 0x00,
        0x00,
    ],
    [
        0xF0, 0x60, 0x20, 0x20, 0x24, 0x20, 0x60, 0x00, 0x60, 0xB0, 0xF0, 0xF0, 0x60, 0x60, 0xE0,
        0xC0,
    ],
];

const RIP_TILE_OFFSETS: [usize; 8] = [
    0x171C0, 0x171D0, 0x171E0, 0x171F0, 0x17200, 0x17210, 0x17220, 0x17230,
];
const RIP_XOR_MASKS: [[u8; TILE_SIZE]; 8] = [
    [
        0x00, 0x00, 0x00, 0x10, 0x10, 0x12, 0x1D, 0x14, 0x00, 0x00, 0x1F, 0x18, 0x11, 0x18, 0x1E,
        0x17,
    ],
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x20, 0x80, 0x00, 0x00, 0xF8, 0x58, 0x08, 0xB8, 0xA8,
        0x48,
    ],
    [
        0x04, 0x10, 0x1E, 0x1A, 0x12, 0x12, 0x13, 0x1A, 0x05, 0x17, 0x17, 0x1A, 0x13, 0x14, 0x10,
        0x1E,
    ],
    [
        0x80, 0x20, 0x00, 0xC0, 0x00, 0x00, 0x80, 0x00, 0xA8, 0x68, 0x28, 0x48, 0xE8, 0x48, 0x00,
        0x60,
    ],
    [
        0x14, 0x10, 0x12, 0x10, 0x12, 0x15, 0x12, 0x02, 0x13, 0x11, 0x12, 0x13, 0x14, 0x14, 0x1A,
        0x06,
    ],
    [
        0x40, 0x00, 0x00, 0x20, 0x40, 0x80, 0x00, 0x00, 0xA8, 0x68, 0x28, 0xF8, 0x60, 0x08, 0x88,
        0x68,
    ],
    [
        0x14, 0x17, 0x16, 0x19, 0x11, 0x10, 0x1F, 0x07, 0x19, 0x18, 0x1E, 0x18, 0x10, 0x10, 0x18,
        0x07,
    ],
    [
        0x00, 0xC0, 0x00, 0x00, 0x80, 0x00, 0xF0, 0xC0, 0x98, 0x38, 0x18, 0xA8, 0x48, 0x78, 0x38,
        0xC0,
    ],
];

#[derive(Debug, Clone)]
pub struct ReferenceLabelsReport {
    pub hit_source_ja: String,
    pub hit_text: String,
    pub rip_source_ja: String,
    pub rip_text: String,
    pub translation_status: String,
    pub tile_count: usize,
}

#[derive(Debug, Clone)]
pub struct ReferenceLabelsBuild {
    pub data: Vec<u8>,
    pub report: ReferenceLabelsReport,
    pub writes: Vec<WriteReport>,
}

pub fn build(source: &Rom, translation: &GraphicsTranslation) -> Result<ReferenceLabelsBuild> {
    source.verify_supported_japanese()?;
    let plan = translation.validate_reference_labels(source)?;
    let baseline = build_expanded(source)?;
    let mut image = TrackedImage::new(baseline.clone());
    let chr_output_base = HEADER_SIZE + EXPANDED_PRG_SIZE;

    let hit_target = compile_target_tiles(source, &HIT_TILE_OFFSETS, &HIT_XOR_MASKS)?;
    ensure!(
        sha1_hex(&hit_target) == HIT_TARGET_SHA1,
        "HIT target tile identity mismatch"
    );
    write_tiles(
        &mut image,
        source,
        chr_output_base,
        "replace baked 当 label with HIT",
        &HIT_TILE_OFFSETS,
        &hit_target,
    )?;

    let rip_target = compile_target_tiles(source, &RIP_TILE_OFFSETS, &RIP_XOR_MASKS)?;
    ensure!(
        sha1_hex(&rip_target) == RIP_TARGET_SHA1,
        "RIP target tile identity mismatch"
    );
    write_tiles(
        &mut image,
        source,
        chr_output_base,
        "replace baked 南無 marker with RIP",
        &RIP_TILE_OFFSETS,
        &rip_target,
    )?;

    image.check_untracked_writes(&baseline)?;
    let writes = image.reports().to_vec();
    Ok(ReferenceLabelsBuild {
        data: image.into_data(),
        report: report(&plan),
        writes,
    })
}

fn compile_target_tiles<const N: usize>(
    source: &Rom,
    offsets: &[usize; N],
    masks: &[[u8; TILE_SIZE]; N],
) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(N * TILE_SIZE);
    for (&offset, mask) in offsets.iter().zip(masks) {
        let end = offset + TILE_SIZE;
        ensure!(
            end <= source.chr().len(),
            "reference label tile is outside CHR"
        );
        output.extend(
            source.chr()[offset..end]
                .iter()
                .zip(mask)
                .map(|(&byte, &xor)| byte ^ xor),
        );
    }
    Ok(output)
}

fn write_tiles(
    image: &mut TrackedImage,
    source: &Rom,
    chr_output_base: usize,
    label: &str,
    offsets: &[usize],
    target: &[u8],
) -> Result<()> {
    ensure!(
        target.len() == offsets.len() * TILE_SIZE,
        "reference label target length mismatch"
    );
    for (index, (&chr_offset, replacement)) in offsets
        .iter()
        .zip(target.chunks_exact(TILE_SIZE))
        .enumerate()
    {
        image.write_expect(
            format!("{label} tile {index}"),
            chr_output_base + chr_offset,
            &source.chr()[chr_offset..chr_offset + TILE_SIZE],
            replacement,
        )?;
    }
    Ok(())
}

fn report(plan: &ReferenceLabelsTranslation) -> ReferenceLabelsReport {
    ReferenceLabelsReport {
        hit_source_ja: plan.hit.source_ja.clone(),
        hit_text: plan.hit.text.clone(),
        rip_source_ja: plan.rip.source_ja.clone(),
        rip_text: plan.rip.text.clone(),
        translation_status: plan.translation_status.clone(),
        tile_count: HIT_TILE_OFFSETS.len() + RIP_TILE_OFFSETS.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_masks_are_nonempty_and_cover_exactly_twelve_tiles() {
        assert_eq!(HIT_TILE_OFFSETS.len() + RIP_TILE_OFFSETS.len(), 12);
        assert!(HIT_XOR_MASKS.iter().flatten().any(|byte| *byte != 0));
        assert!(RIP_XOR_MASKS.iter().flatten().any(|byte| *byte != 0));
    }

    #[test]
    fn tile_offsets_are_ordered_and_disjoint() {
        let mut offsets = HIT_TILE_OFFSETS
            .into_iter()
            .chain(RIP_TILE_OFFSETS)
            .collect::<Vec<_>>();
        let original = offsets.clone();
        offsets.sort_unstable();
        offsets.dedup();
        assert_eq!(offsets.len(), original.len());
        assert_eq!(offsets, original);
    }
}
