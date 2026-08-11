use anyhow::{Result, ensure};

use crate::{
    graphics_translation::GraphicsTranslation, rom::Rom, tracked::WriteReport,
    translation::EndingTranslation,
};

use super::{
    build_expanded, build_expanded_chr,
    ending_draft_poc::{self, EndingDraftPocReport},
    reference_labels::{self, ReferenceLabelsReport},
    roulette_label::{self, RouletteLabelReport},
    title_poc::{self, TitlePocReport},
};

#[derive(Debug, Clone)]
pub struct KoreanDraftReport {
    pub title: TitlePocReport,
    pub roulette_label: RouletteLabelReport,
    pub reference_labels: ReferenceLabelsReport,
    pub ending: EndingDraftPocReport,
}

#[derive(Debug, Clone)]
pub struct KoreanDraftBuild {
    pub data: Vec<u8>,
    pub report: KoreanDraftReport,
    pub writes: Vec<WriteReport>,
}

pub fn build(
    source: &Rom,
    title_asset_png: &[u8],
    graphics_translation: &GraphicsTranslation,
    ending_translation: &EndingTranslation,
) -> Result<KoreanDraftBuild> {
    source.verify_supported_japanese()?;

    // Both component stages derive in-memory deltas from the verified Japanese
    // ROM. The caller cannot supply a PoC ROM or another generated artifact.
    let title = title_poc::build(source, title_asset_png)?;
    let roulette_label = roulette_label::build(source, graphics_translation)?;
    let reference_labels = reference_labels::build(source, graphics_translation)?;
    let ending = ending_draft_poc::build(source, ending_translation)?;
    let title_baseline = build_expanded(source)?;
    let integrated_baseline = build_expanded_chr(source)?;
    let title_and_ending = merge_verified_deltas(
        &title_baseline,
        &title.data,
        &integrated_baseline,
        &ending.data,
    )?;
    let baked_labels = merge_verified_deltas(
        &title_baseline,
        &roulette_label.data,
        &title_baseline,
        &reference_labels.data,
    )?;
    let data = merge_verified_deltas(
        &title_baseline,
        &baked_labels,
        &integrated_baseline,
        &title_and_ending,
    )?;

    let mut writes = namespace_writes("title", title.writes);
    writes.extend(namespace_writes("roulette-label", roulette_label.writes));
    writes.extend(namespace_writes(
        "reference-labels",
        reference_labels.writes,
    ));
    writes.extend(namespace_writes("ending", ending.writes));

    Ok(KoreanDraftBuild {
        data,
        report: KoreanDraftReport {
            title: title.report,
            roulette_label: roulette_label.report,
            reference_labels: reference_labels.report,
            ending: ending.report,
        },
        writes,
    })
}

fn namespace_writes(namespace: &str, writes: Vec<WriteReport>) -> Vec<WriteReport> {
    writes
        .into_iter()
        .map(|write| WriteReport {
            label: format!("{namespace}: {}", write.label),
            offset: write.offset,
            len: write.len,
        })
        .collect()
}

fn merge_verified_deltas(
    smaller_baseline: &[u8],
    smaller_output: &[u8],
    full_baseline: &[u8],
    full_output: &[u8],
) -> Result<Vec<u8>> {
    ensure!(
        smaller_output.len() == smaller_baseline.len(),
        "component output size does not match its baseline"
    );
    ensure!(
        full_output.len() == full_baseline.len(),
        "integrated component output size does not match its baseline"
    );
    ensure!(
        smaller_baseline.len() <= full_baseline.len(),
        "component baseline is larger than the integrated baseline"
    );
    let mut merged = full_output.to_vec();
    for (offset, (&before, &after)) in smaller_baseline.iter().zip(smaller_output).enumerate() {
        if before == after {
            continue;
        }
        ensure!(
            full_baseline[offset] == before,
            "component delta baseline mismatch at {offset:#X}: {before:02X} versus {:02X}",
            full_baseline[offset]
        );
        let other = full_output[offset];
        ensure!(
            other == full_baseline[offset] || other == after,
            "component write conflict at {offset:#X}: {before:02X} -> {after:02X} versus {other:02X}"
        );
        merged[offset] = after;
    }

    for (offset, (&before, &after)) in full_baseline.iter().zip(&merged).enumerate() {
        if before != after {
            let from_smaller = smaller_baseline
                .get(offset)
                .zip(smaller_output.get(offset))
                .is_some_and(|(&component_before, &component_after)| {
                    component_before != component_after && component_after == after
                });
            let from_full = full_output[offset] != before && full_output[offset] == after;
            ensure!(
                from_smaller || from_full,
                "unattributed integrated write at {offset:#X}: {before:02X} -> {after:02X}"
            );
        }
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_disjoint_component_deltas_over_a_shared_prefix() {
        let merged =
            merge_verified_deltas(&[0, 0, 0], &[1, 0, 0], &[0, 0, 0, 0, 0], &[0, 2, 0, 3, 0])
                .unwrap();

        assert_eq!(merged, vec![1, 2, 0, 3, 0]);
    }

    #[test]
    fn accepts_identical_overlapping_deltas() {
        let merged = merge_verified_deltas(&[0], &[1], &[0, 0], &[1, 0]).unwrap();
        assert_eq!(merged, vec![1, 0]);
    }

    #[test]
    fn rejects_conflicting_component_deltas() {
        let error = merge_verified_deltas(&[0], &[1], &[0, 0], &[2, 0]).unwrap_err();
        assert!(error.to_string().contains("write conflict at 0x0"));
    }

    #[test]
    fn accepts_unmodified_header_differences_between_baselines() {
        let merged = merge_verified_deltas(&[0], &[0], &[9, 0], &[9, 1]).unwrap();
        assert_eq!(merged, vec![9, 1]);
    }

    #[test]
    fn rejects_a_divergent_baseline_under_a_component_delta() {
        let error = merge_verified_deltas(&[0], &[1], &[9, 0], &[9, 0]).unwrap_err();
        assert!(error.to_string().contains("delta baseline mismatch"));
    }
}
