use std::fs;

use anyhow::{Context, Result, ensure};
use clap::Parser;
use nes_parodius_patch::{
    bps, build, cli, ending, font, graphics_translation, rom, sha1_hex, translation,
};

fn main() -> Result<()> {
    let args = cli::Cli::parse();
    match args.command {
        cli::Command::Info { rom: path } => {
            let image = rom::Rom::from_path(&path)?;
            println!("path={}", path.display());
            println!("sha1={}", sha1_hex(image.data()));
            println!("format={}", image.format_name());
            println!("mapper={}.{}", image.mapper(), image.submapper());
            println!("prg_size={}", image.prg().len());
            println!("chr_size={}", image.chr().len());
            Ok(())
        }
        cli::Command::BuildExpanded { rom: path, output } => {
            let source = rom::Rom::from_path(&path)?;
            let built = build::build_expanded(&source)?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create output directory {}", parent.display()))?;
            }
            fs::write(&output, &built)
                .with_context(|| format!("write expanded ROM {}", output.display()))?;
            println!("verified_source_sha1={}", sha1_hex(source.data()));
            println!("size={}", built.len());
            println!("sha1={}", sha1_hex(&built));
            println!("{}", output.display());
            Ok(())
        }
        cli::Command::BuildEndingPoc {
            rom: path,
            spec: spec_path,
            output,
            preview,
        } => {
            let source = rom::Rom::from_path(&path)?;
            let spec = font::PocSpec::from_path(&spec_path)?;
            let result = build::ending_poc::build(&source, &spec)?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create output directory {}", parent.display()))?;
            }
            fs::write(&output, &result.data)
                .with_context(|| format!("write PoC ROM {}", output.display()))?;
            if let Some(preview_path) = preview {
                font::write_preview(&preview_path, &spec, 8)?;
                println!("preview={}", preview_path.display());
            }
            println!("verified_source_sha1={}", sha1_hex(source.data()));
            println!("text={}", result.report.text);
            println!("glyph_count={}", result.report.glyph_count);
            println!("blanked_records={}", result.report.blanked_records);
            println!("ending_used={}", result.report.used);
            println!("ending_remaining={}", result.report.remaining);
            for write in &result.writes {
                println!(
                    "write={} offset={:#X} len={}",
                    write.label, write.offset, write.len
                );
            }
            println!("size={}", result.data.len());
            println!("sha1={}", sha1_hex(&result.data));
            println!("{}", output.display());
            Ok(())
        }
        cli::Command::BuildTitlePoc {
            rom: path,
            asset,
            output,
            preview,
        } => {
            let source = rom::Rom::from_path(&path)?;
            let asset_png = fs::read(&asset)
                .with_context(|| format!("read title asset {}", asset.display()))?;
            let result = build::title_poc::build(&source, &asset_png)?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create output directory {}", parent.display()))?;
            }
            fs::write(&output, &result.data)
                .with_context(|| format!("write title PoC ROM {}", output.display()))?;
            if let Some(preview_path) = preview {
                build::title_poc::write_preview(&preview_path, &asset_png, 4)?;
                println!("preview={}", preview_path.display());
            }
            println!("verified_source_sha1={}", sha1_hex(source.data()));
            println!(
                "asset={}x{} bounds={},{},{},{}",
                result.report.source_width,
                result.report.source_height,
                result.report.source_bounds.0,
                result.report.source_bounds.1,
                result.report.source_bounds.2,
                result.report.source_bounds.3
            );
            println!(
                "title_tiles={}/{}",
                result.report.tile_count, result.report.tile_capacity
            );
            for write in &result.writes {
                println!(
                    "write={} offset={:#X} len={}",
                    write.label, write.offset, write.len
                );
            }
            println!("size={}", result.data.len());
            println!("sha1={}", sha1_hex(&result.data));
            println!("{}", output.display());
            Ok(())
        }
        cli::Command::ExtractEndingCorpus { rom: path, output } => {
            let source = rom::Rom::from_path(&path)?;
            let manifest = ending::extract_manifest(&source)?;
            let mut encoded =
                serde_json::to_vec_pretty(&manifest).context("serialize Japanese ending corpus")?;
            encoded.push(b'\n');
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create output directory {}", parent.display()))?;
            }
            fs::write(&output, encoded)
                .with_context(|| format!("write ending corpus {}", output.display()))?;
            println!("verified_source_sha1={}", sha1_hex(source.data()));
            println!("{}", output.display());
            Ok(())
        }
        cli::Command::PlanEndingTranslation {
            rom: path,
            translation: translation_path,
            output,
        } => {
            let source = rom::Rom::from_path(&path)?;
            let spec = translation::EndingTranslation::from_path(&translation_path)?;
            let plan = translation::plan(&source, &spec)?;
            let mut encoded =
                serde_json::to_vec_pretty(&plan).context("serialize ending translation plan")?;
            encoded.push(b'\n');
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create output directory {}", parent.display()))?;
            }
            fs::write(&output, encoded)
                .with_context(|| format!("write translation plan {}", output.display()))?;
            println!("verified_source_sha1={}", sha1_hex(source.data()));
            println!("translation_status={}", plan.translation_status);
            println!("entries={}", plan.entries.len());
            println!("unique_glyphs={}", plan.unique_glyphs);
            println!("font_pages={}", plan.pages.len());
            println!("{}", output.display());
            Ok(())
        }
        cli::Command::BuildEndingDraftPoc {
            rom: path,
            translation: translation_path,
            output,
        } => {
            let source = rom::Rom::from_path(&path)?;
            let spec = translation::EndingTranslation::from_path(&translation_path)?;
            let result = build::ending_draft_poc::build(&source, &spec)?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create output directory {}", parent.display()))?;
            }
            fs::write(&output, &result.data)
                .with_context(|| format!("write ending draft PoC {}", output.display()))?;
            println!("verified_source_sha1={}", sha1_hex(source.data()));
            println!("translation_status={}", result.report.translation_status);
            println!("entries={}", result.report.entries);
            println!("unique_glyphs={}", result.report.unique_glyphs);
            println!("font_pages={}", result.report.font_pages);
            println!("ending_used={}", result.report.ending_used);
            println!("ending_remaining={}", result.report.ending_remaining);
            for write in &result.writes {
                println!(
                    "write={} offset={:#X} len={}",
                    write.label, write.offset, write.len
                );
            }
            println!("size={}", result.data.len());
            println!("sha1={}", sha1_hex(&result.data));
            println!("{}", output.display());
            Ok(())
        }
        cli::Command::BuildKoreanDraft {
            rom: path,
            title_asset,
            graphics_translation: graphics_translation_path,
            ending_translation,
            output,
            bps_output,
            bps_ines_header_output,
        } => {
            let source = rom::Rom::from_path(&path)?;
            let asset_png = fs::read(&title_asset)
                .with_context(|| format!("read title asset {}", title_asset.display()))?;
            let graphics_translation =
                graphics_translation::GraphicsTranslation::from_path(&graphics_translation_path)?;
            let translation = translation::EndingTranslation::from_path(&ending_translation)?;
            let result = build::korean_draft::build(
                &source,
                &asset_png,
                &graphics_translation,
                &translation,
            )?;
            let verified_bps = if let Some(path) = &bps_output {
                ensure!(
                    path != &output,
                    "integrated ROM and BPS output paths must differ"
                );
                Some(bps::create_verified_with_mappings(
                    source.payload(),
                    &result.data,
                    &build::headerless_bps_mappings(),
                )?)
            } else {
                None
            };
            let verified_ines_header_bps = if let Some(path) = &bps_ines_header_output {
                ensure!(
                    path != &output,
                    "integrated ROM and iNES-header BPS output paths must differ"
                );
                if let Some(headerless_path) = &bps_output {
                    ensure!(
                        path != headerless_path,
                        "headerless and iNES-header BPS output paths must differ"
                    );
                }
                Some(bps::create_verified(source.data(), &result.data)?)
            } else {
                None
            };
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create output directory {}", parent.display()))?;
            }
            fs::write(&output, &result.data)
                .with_context(|| format!("write integrated Korean draft {}", output.display()))?;
            if let (Some(path), Some(patch)) = (&bps_output, &verified_bps) {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("create BPS output directory {}", parent.display())
                    })?;
                }
                fs::write(path, patch.bytes())
                    .with_context(|| format!("write verified BPS patch {}", path.display()))?;
            }
            if let (Some(path), Some(patch)) = (&bps_ines_header_output, &verified_ines_header_bps)
            {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "create iNES-header BPS output directory {}",
                            parent.display()
                        )
                    })?;
                }
                fs::write(path, patch.bytes()).with_context(|| {
                    format!("write verified iNES-header BPS patch {}", path.display())
                })?;
            }
            println!("verified_source_sha1={}", sha1_hex(source.data()));
            println!(
                "title_tiles={}/{}",
                result.report.title.tile_count, result.report.title.tile_capacity
            );
            println!(
                "roulette_label={} source={} reference={} tiles={} status={} font={} size={} threshold={} body={}x{} sprite_spacing={}",
                result.report.roulette_label.text,
                result.report.roulette_label.source_ja,
                result.report.roulette_label.reference_en,
                result.report.roulette_label.tile_count,
                result.report.roulette_label.translation_status,
                result.report.roulette_label.font,
                result.report.roulette_label.font_size,
                result.report.roulette_label.threshold,
                result.report.roulette_label.body_width,
                result.report.roulette_label.body_height,
                result.report.roulette_label.sprite_spacing
            );
            println!(
                "reference_labels={}=>{},{}=>{} tiles={} status={}",
                result.report.reference_labels.hit_source_ja,
                result.report.reference_labels.hit_text,
                result.report.reference_labels.rip_source_ja,
                result.report.reference_labels.rip_text,
                result.report.reference_labels.tile_count,
                result.report.reference_labels.translation_status
            );
            println!(
                "translation_status={}",
                result.report.ending.translation_status
            );
            println!("ending_entries={}", result.report.ending.entries);
            println!("unique_glyphs={}", result.report.ending.unique_glyphs);
            println!("font_pages={}", result.report.ending.font_pages);
            println!("ending_used={}", result.report.ending.ending_used);
            println!("ending_remaining={}", result.report.ending.ending_remaining);
            for write in &result.writes {
                println!(
                    "write={} offset={:#X} len={}",
                    write.label, write.offset, write.len
                );
            }
            println!("size={}", result.data.len());
            println!("sha1={}", sha1_hex(&result.data));
            if let (Some(path), Some(patch)) = (&bps_output, &verified_bps) {
                let report = patch.report();
                println!("bps_format=BPS1");
                println!("bps_size={}", report.patch_len);
                println!("bps_source_crc32={:08X}", report.source_crc32);
                println!("bps_target_crc32={:08X}", report.target_crc32);
                println!("bps_patch_crc32={:08X}", report.patch_crc32);
                println!("bps_source_sha1={}", report.source_sha1);
                println!("bps_target_sha1={}", report.target_sha1);
                println!("bps_patch_sha1={}", report.patch_sha1);
                println!("bps_apply_roundtrip_exact={}", report.apply_roundtrip_exact);
                println!("bps={}", path.display());
            }
            if let (Some(path), Some(patch)) = (&bps_ines_header_output, &verified_ines_header_bps)
            {
                let report = patch.report();
                println!("ines_header_bps_format=BPS1");
                println!("ines_header_bps_size={}", report.patch_len);
                println!("ines_header_bps_source_crc32={:08X}", report.source_crc32);
                println!("ines_header_bps_target_crc32={:08X}", report.target_crc32);
                println!("ines_header_bps_patch_crc32={:08X}", report.patch_crc32);
                println!("ines_header_bps_source_sha1={}", report.source_sha1);
                println!("ines_header_bps_target_sha1={}", report.target_sha1);
                println!("ines_header_bps_patch_sha1={}", report.patch_sha1);
                println!(
                    "ines_header_bps_apply_roundtrip_exact={}",
                    report.apply_roundtrip_exact
                );
                println!("ines_header_bps={}", path.display());
            }
            println!("{}", output.display());
            Ok(())
        }
        cli::Command::SweepRouletteLabel {
            text,
            font: font_paths,
            size: sizes,
            threshold,
            letter_spacing,
            shade_split,
            output,
        } => {
            let sizes = if sizes.is_empty() {
                build::roulette_sweep::DEFAULT_SIZES.to_vec()
            } else {
                sizes
            };
            let report = build::roulette_sweep::write_sweep(
                &output,
                &text,
                &font_paths,
                &sizes,
                threshold,
                letter_spacing,
                shade_split,
            )?;
            println!("text={}", report.text);
            println!("canvas={}x{}", report.width, report.height);
            println!("palette=0F,30,28,06");
            println!("threshold={}", report.threshold);
            println!("letter_spacing={}", report.letter_spacing);
            println!("shade_split={}", report.shade_split);
            for candidate in &report.candidates {
                println!(
                    "candidate={} size={} fit={} body={}x{} chr_sha1={} reason={}",
                    candidate.font,
                    candidate.size,
                    candidate.fit,
                    candidate.body_width,
                    candidate.body_height,
                    candidate.chr_sha1.as_deref().unwrap_or("-"),
                    candidate.reason.as_deref().unwrap_or("-")
                );
            }
            println!("{}", output.display());
            Ok(())
        }
        cli::Command::ApplyBps {
            rom: path,
            patch,
            output,
        } => {
            let source =
                fs::read(&path).with_context(|| format!("read BPS source {}", path.display()))?;
            let patch_bytes =
                fs::read(&patch).with_context(|| format!("read BPS patch {}", patch.display()))?;
            let target = bps::apply(&source, &patch_bytes)?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create output directory {}", parent.display()))?;
            }
            fs::write(&output, &target)
                .with_context(|| format!("write BPS-applied ROM {}", output.display()))?;
            println!("verified_source_sha1={}", sha1_hex(&source));
            println!("bps_sha1={}", sha1_hex(&patch_bytes));
            println!("size={}", target.len());
            println!("sha1={}", sha1_hex(&target));
            println!("{}", output.display());
            Ok(())
        }
    }
}
