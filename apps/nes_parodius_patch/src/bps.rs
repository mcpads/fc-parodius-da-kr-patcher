//! Deterministic BPS1 generation and strict application.
//!
//! The aligned encoder emits SourceRead and TargetRead actions. The mapped
//! encoder additionally emits SourceCopy actions so a headerless preservation
//! source can produce the same canonical headered target without duplicating
//! unchanged PRG and CHR ranges in the patch.

use anyhow::{Context, Result, ensure};

use crate::sha1_hex;

const MAGIC: &[u8; 4] = b"BPS1";
const FOOTER_LEN: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BpsPatchReport {
    pub source_len: usize,
    pub target_len: usize,
    pub patch_len: usize,
    pub source_crc32: u32,
    pub target_crc32: u32,
    pub patch_crc32: u32,
    pub source_sha1: String,
    pub target_sha1: String,
    pub patch_sha1: String,
    pub apply_roundtrip_exact: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBpsPatch {
    bytes: Vec<u8>,
    report: BpsPatchReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceMapping {
    pub source_start: usize,
    pub target_start: usize,
    pub len: usize,
}

impl SourceMapping {
    pub const fn new(source_start: usize, target_start: usize, len: usize) -> Self {
        Self {
            source_start,
            target_start,
            len,
        }
    }
}

impl VerifiedBpsPatch {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn report(&self) -> &BpsPatchReport {
        &self.report
    }
}

#[derive(Debug)]
enum Action {
    SourceRead(usize),
    TargetRead(Vec<u8>),
    SourceCopy { source_start: usize, len: usize },
}

pub fn create_verified(source: &[u8], target: &[u8]) -> Result<VerifiedBpsPatch> {
    let patch = generate(source, target);
    verify_generated(source, target, patch)
}

pub fn create_verified_with_mappings(
    source: &[u8],
    target: &[u8],
    mappings: &[SourceMapping],
) -> Result<VerifiedBpsPatch> {
    let patch = generate_with_mappings(source, target, mappings)?;
    verify_generated(source, target, patch)
}

fn verify_generated(source: &[u8], target: &[u8], patch: Vec<u8>) -> Result<VerifiedBpsPatch> {
    let applied = apply(source, &patch)?;
    ensure!(
        applied == target,
        "BPS self-check failed: applied output differs from target"
    );
    let footer = footer(&patch)?;
    Ok(VerifiedBpsPatch {
        report: BpsPatchReport {
            source_len: source.len(),
            target_len: target.len(),
            patch_len: patch.len(),
            source_crc32: footer.source_crc32,
            target_crc32: footer.target_crc32,
            patch_crc32: footer.patch_crc32,
            source_sha1: sha1_hex(source),
            target_sha1: sha1_hex(target),
            patch_sha1: sha1_hex(&patch),
            apply_roundtrip_exact: true,
        },
        bytes: patch,
    })
}

pub fn generate(source: &[u8], target: &[u8]) -> Vec<u8> {
    encode_patch(source, target, generate_actions(source, target))
        .expect("aligned BPS generation cannot produce invalid source-copy offsets")
}

fn generate_with_mappings(
    source: &[u8],
    target: &[u8],
    mappings: &[SourceMapping],
) -> Result<Vec<u8>> {
    validate_mappings(source, target, mappings)?;
    encode_patch(
        source,
        target,
        generate_mapped_actions(source, target, mappings),
    )
}

fn encode_patch(source: &[u8], target: &[u8], actions: Vec<Action>) -> Result<Vec<u8>> {
    let mut patch = Vec::new();
    patch.extend_from_slice(MAGIC);
    encode_number(&mut patch, source.len() as u64);
    encode_number(&mut patch, target.len() as u64);
    encode_number(&mut patch, 0);
    let mut source_relative_offset = 0_i64;
    for action in actions {
        encode_action(&mut patch, &action, &mut source_relative_offset)?;
    }
    patch.extend_from_slice(&crc32(source).to_le_bytes());
    patch.extend_from_slice(&crc32(target).to_le_bytes());
    let patch_crc32 = crc32(&patch);
    patch.extend_from_slice(&patch_crc32.to_le_bytes());
    Ok(patch)
}

pub fn apply(source: &[u8], patch: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        patch.len() >= MAGIC.len() + FOOTER_LEN,
        "BPS patch is too small"
    );
    ensure!(&patch[..MAGIC.len()] == MAGIC, "BPS magic is not BPS1");
    let footer = footer(patch)?;
    ensure!(
        footer.patch_crc32 == crc32(&patch[..patch.len() - 4]),
        "BPS patch CRC mismatch"
    );

    let footer_start = patch.len() - FOOTER_LEN;
    let mut cursor = MAGIC.len();
    let source_size = decode_number(patch, &mut cursor, footer_start)?;
    let target_size = decode_number(patch, &mut cursor, footer_start)?;
    let metadata_size = usize::try_from(decode_number(patch, &mut cursor, footer_start)?)
        .context("BPS metadata length does not fit this host")?;
    cursor = cursor
        .checked_add(metadata_size)
        .context("BPS metadata range overflows")?;
    ensure!(
        cursor <= footer_start,
        "BPS metadata exceeds the patch body"
    );
    ensure!(
        source_size == source.len() as u64,
        "BPS source size mismatch: expected {source_size}, found {}",
        source.len()
    );
    ensure!(
        footer.source_crc32 == crc32(source),
        "BPS source CRC mismatch"
    );

    let target_len = usize::try_from(target_size).context("BPS target length does not fit host")?;
    let mut target = vec![0; target_len];
    let mut output_offset = 0_usize;
    let mut source_relative_offset = 0_i64;
    let mut target_relative_offset = 0_i64;

    while cursor < footer_start {
        let encoded = decode_number(patch, &mut cursor, footer_start)?;
        let length =
            usize::try_from((encoded >> 2) + 1).context("BPS action length does not fit host")?;
        let output_end = output_offset
            .checked_add(length)
            .context("BPS output range overflows")?;
        ensure!(output_end <= target.len(), "BPS action exceeds target size");

        match encoded & 3 {
            0 => {
                ensure!(
                    output_end <= source.len(),
                    "BPS SourceRead exceeds source size"
                );
                target[output_offset..output_end]
                    .copy_from_slice(&source[output_offset..output_end]);
                output_offset = output_end;
            }
            1 => {
                let patch_end = cursor
                    .checked_add(length)
                    .context("BPS TargetRead range overflows")?;
                ensure!(
                    patch_end <= footer_start,
                    "BPS TargetRead exceeds patch body"
                );
                target[output_offset..output_end].copy_from_slice(&patch[cursor..patch_end]);
                cursor = patch_end;
                output_offset = output_end;
            }
            2 => {
                let delta = decode_signed_delta(patch, &mut cursor, footer_start)?;
                source_relative_offset = source_relative_offset
                    .checked_add(delta)
                    .context("BPS SourceCopy relative offset overflows")?;
                for output in &mut target[output_offset..output_end] {
                    let source_index = usize::try_from(source_relative_offset)
                        .context("BPS SourceCopy uses a negative source offset")?;
                    *output = *source
                        .get(source_index)
                        .context("BPS SourceCopy exceeds source size")?;
                    source_relative_offset = source_relative_offset
                        .checked_add(1)
                        .context("BPS SourceCopy cursor overflows")?;
                }
                output_offset = output_end;
            }
            3 => {
                let delta = decode_signed_delta(patch, &mut cursor, footer_start)?;
                target_relative_offset = target_relative_offset
                    .checked_add(delta)
                    .context("BPS TargetCopy relative offset overflows")?;
                while output_offset < output_end {
                    let target_index = usize::try_from(target_relative_offset)
                        .context("BPS TargetCopy uses a negative target offset")?;
                    ensure!(
                        target_index < output_offset,
                        "BPS TargetCopy reads data that has not been produced"
                    );
                    target[output_offset] = target[target_index];
                    output_offset += 1;
                    target_relative_offset = target_relative_offset
                        .checked_add(1)
                        .context("BPS TargetCopy cursor overflows")?;
                }
            }
            _ => unreachable!(),
        }
    }
    ensure!(
        output_offset == target.len(),
        "BPS output size mismatch: wrote {output_offset}, expected {}",
        target.len()
    );
    ensure!(
        footer.target_crc32 == crc32(&target),
        "BPS target CRC mismatch"
    );
    Ok(target)
}

fn generate_actions(source: &[u8], target: &[u8]) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut position = 0;
    while position < target.len() {
        if position < source.len() && source[position] == target[position] {
            let start = position;
            while position < target.len()
                && position < source.len()
                && source[position] == target[position]
            {
                position += 1;
            }
            actions.push(Action::SourceRead(position - start));
        } else {
            let start = position;
            while position < target.len()
                && (position >= source.len() || source[position] != target[position])
            {
                position += 1;
            }
            actions.push(Action::TargetRead(target[start..position].to_vec()));
        }
    }
    actions
}

fn validate_mappings(source: &[u8], target: &[u8], mappings: &[SourceMapping]) -> Result<()> {
    let mut previous_target_end = 0_usize;
    for mapping in mappings {
        ensure!(mapping.len > 0, "BPS source mapping must not be empty");
        let source_end = mapping
            .source_start
            .checked_add(mapping.len)
            .context("BPS source mapping range overflows")?;
        let target_end = mapping
            .target_start
            .checked_add(mapping.len)
            .context("BPS target mapping range overflows")?;
        ensure!(
            source_end <= source.len(),
            "BPS source mapping exceeds source size"
        );
        ensure!(
            target_end <= target.len(),
            "BPS source mapping exceeds target size"
        );
        ensure!(
            mapping.target_start >= previous_target_end,
            "BPS source mappings must be ordered and non-overlapping"
        );
        previous_target_end = target_end;
    }
    Ok(())
}

fn generate_mapped_actions(
    source: &[u8],
    target: &[u8],
    mappings: &[SourceMapping],
) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut target_position = 0_usize;

    for mapping in mappings {
        if target_position < mapping.target_start {
            push_target_read(&mut actions, &target[target_position..mapping.target_start]);
        }

        let target_end = mapping.target_start + mapping.len;
        target_position = mapping.target_start;
        while target_position < target_end {
            let source_position = mapping.source_start + (target_position - mapping.target_start);
            if source[source_position] == target[target_position] {
                let source_start = source_position;
                let run_start = target_position;
                while target_position < target_end {
                    let mapped_source =
                        mapping.source_start + (target_position - mapping.target_start);
                    if source[mapped_source] != target[target_position] {
                        break;
                    }
                    target_position += 1;
                }
                actions.push(Action::SourceCopy {
                    source_start,
                    len: target_position - run_start,
                });
            } else {
                let run_start = target_position;
                while target_position < target_end {
                    let mapped_source =
                        mapping.source_start + (target_position - mapping.target_start);
                    if source[mapped_source] == target[target_position] {
                        break;
                    }
                    target_position += 1;
                }
                push_target_read(&mut actions, &target[run_start..target_position]);
            }
        }
    }

    if target_position < target.len() {
        push_target_read(&mut actions, &target[target_position..]);
    }
    actions
}

fn push_target_read(actions: &mut Vec<Action>, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    if let Some(Action::TargetRead(previous)) = actions.last_mut() {
        previous.extend_from_slice(bytes);
    } else {
        actions.push(Action::TargetRead(bytes.to_vec()));
    }
}

fn encode_action(
    patch: &mut Vec<u8>,
    action: &Action,
    source_relative_offset: &mut i64,
) -> Result<()> {
    match action {
        Action::SourceRead(length) => encode_number(patch, (*length as u64 - 1) << 2),
        Action::TargetRead(bytes) => {
            encode_number(patch, ((bytes.len() as u64 - 1) << 2) | 1);
            patch.extend_from_slice(bytes);
        }
        Action::SourceCopy { source_start, len } => {
            encode_number(patch, ((*len as u64 - 1) << 2) | 2);
            let source_start =
                i64::try_from(*source_start).context("BPS source-copy offset exceeds i64")?;
            let delta = source_start
                .checked_sub(*source_relative_offset)
                .context("BPS source-copy delta overflows")?;
            encode_signed_delta(patch, delta)?;
            let len = i64::try_from(*len).context("BPS source-copy length exceeds i64")?;
            *source_relative_offset = source_start
                .checked_add(len)
                .context("BPS source-copy cursor overflows")?;
        }
    }
    Ok(())
}

fn encode_number(output: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            output.push(byte | 0x80);
            return;
        }
        output.push(byte);
        value -= 1;
    }
}

fn decode_number(input: &[u8], cursor: &mut usize, limit: usize) -> Result<u64> {
    let mut value = 0_u64;
    let mut shift = 1_u64;
    loop {
        ensure!(*cursor < limit, "unexpected end of BPS number");
        let byte = input[*cursor];
        *cursor += 1;
        value = value
            .checked_add(
                u64::from(byte & 0x7F)
                    .checked_mul(shift)
                    .context("BPS number overflows")?,
            )
            .context("BPS number overflows")?;
        if byte & 0x80 != 0 {
            return Ok(value);
        }
        shift = shift
            .checked_mul(128)
            .context("BPS number shift overflows")?;
        value = value.checked_add(shift).context("BPS number overflows")?;
    }
}

fn decode_signed_delta(input: &[u8], cursor: &mut usize, limit: usize) -> Result<i64> {
    let encoded = decode_number(input, cursor, limit)?;
    let magnitude = i64::try_from(encoded >> 1).context("BPS relative delta exceeds i64")?;
    Ok(if encoded & 1 == 0 {
        magnitude
    } else {
        -magnitude
    })
}

fn encode_signed_delta(output: &mut Vec<u8>, delta: i64) -> Result<()> {
    let magnitude = delta.unsigned_abs();
    let encoded = magnitude
        .checked_mul(2)
        .and_then(|value| value.checked_add(u64::from(delta < 0)))
        .context("BPS signed delta overflows")?;
    encode_number(output, encoded);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Footer {
    source_crc32: u32,
    target_crc32: u32,
    patch_crc32: u32,
}

fn footer(patch: &[u8]) -> Result<Footer> {
    ensure!(patch.len() >= FOOTER_LEN, "BPS patch has no footer");
    let start = patch.len() - FOOTER_LEN;
    Ok(Footer {
        source_crc32: u32::from_le_bytes(patch[start..start + 4].try_into()?),
        target_crc32: u32::from_le_bytes(patch[start + 4..start + 8].try_into()?),
        patch_crc32: u32::from_le_bytes(patch[start + 8..start + 12].try_into()?),
    })
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn bps_numbers_roundtrip_at_boundaries() {
        for value in [0, 1, 127, 128, 255, 256, 16_383, 16_384, u32::MAX as u64] {
            let mut encoded = Vec::new();
            encode_number(&mut encoded, value);
            let mut cursor = 0;
            assert_eq!(
                decode_number(&encoded, &mut cursor, encoded.len()).unwrap(),
                value
            );
            assert_eq!(cursor, encoded.len());
        }
    }

    #[test]
    fn verified_patch_roundtrips_sparse_rom_changes() {
        let mut source = vec![0xFF; 0x1_0000];
        for index in (0..source.len()).step_by(0x100) {
            source[index] = (index >> 8) as u8;
        }
        let mut target = source.clone();
        for index in 0..16 {
            target[index * 0x1000 + 0x42] = 0xAA;
        }

        let patch = create_verified(&source, &target).unwrap();
        assert_eq!(&patch.bytes()[..4], MAGIC);
        assert_eq!(apply(&source, patch.bytes()).unwrap(), target);
        assert!(patch.bytes().len() < source.len() / 10);
    }

    #[test]
    fn verified_patch_supports_size_changes_and_identical_inputs() {
        let source = b"source";
        for target in [b"source".as_slice(), b"a longer target".as_slice()] {
            let patch = create_verified(source, target).unwrap();
            assert_eq!(apply(source, patch.bytes()).unwrap(), target);
        }
    }

    #[test]
    fn verified_patch_is_deterministic() {
        let source = b"source image";
        let target = b"target image with growth";
        assert_eq!(generate(source, target), generate(source, target));
    }

    #[test]
    fn verified_mapped_patch_inserts_prefix_and_reuses_moved_source_ranges() {
        let source = b"abcdefgh12345678";
        let target = b"HEADabcdWXYZ12345678TAIL";
        let mappings = [SourceMapping::new(0, 4, 4), SourceMapping::new(8, 12, 8)];

        let patch = create_verified_with_mappings(source, target, &mappings).unwrap();
        assert_eq!(apply(source, patch.bytes()).unwrap(), target);
        assert!(patch.bytes().len() < generate(source, target).len());
    }

    #[test]
    fn mapped_patch_rejects_invalid_ranges() {
        let source = b"source";
        let target = b"target";
        let overlapping = [SourceMapping::new(0, 0, 3), SourceMapping::new(3, 2, 3)];
        assert!(create_verified_with_mappings(source, target, &overlapping).is_err());

        let out_of_bounds = [SourceMapping::new(4, 0, 4)];
        assert!(create_verified_with_mappings(source, target, &out_of_bounds).is_err());
    }

    #[test]
    fn apply_rejects_wrong_source_and_corrupt_patch() {
        let patch = generate(b"source", b"target");
        assert!(apply(b"wrong!", &patch).is_err());

        let mut corrupt = patch;
        corrupt[5] ^= 1;
        assert!(apply(b"source", &corrupt).is_err());
    }
}
