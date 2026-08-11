use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "nes-parodius-patch")]
#[command(about = "Independent Rust pipeline for the Parodius Da! NES Korean patch")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show parsed ROM identity and layout information.
    Info {
        /// Input NES ROM.
        rom: PathBuf,
    },
    /// Build the verified 256 KiB Japanese PRG skeleton.
    BuildExpanded {
        /// Supported Japanese ROM.
        rom: PathBuf,
        /// Local full-ROM output used for development verification.
        #[arg(long)]
        output: PathBuf,
    },
    /// Build the isolated Korean ending visibility PoC.
    BuildEndingPoc {
        /// Supported Japanese ROM.
        rom: PathBuf,
        /// Commit-friendly JSON mapping the PoC text to Dalmoori glyph slots.
        #[arg(long)]
        spec: PathBuf,
        /// Local full-ROM output used for emulator verification.
        #[arg(long)]
        output: PathBuf,
        /// Optional PNG rendering of the encoded glyph sequence.
        #[arg(long)]
        preview: Option<PathBuf>,
    },
    /// Build the Korean baked-title graphic PoC from a committed PNG source.
    BuildTitlePoc {
        /// Supported Japanese ROM.
        rom: PathBuf,
        /// RGBA PNG source for the Korean title graphic.
        #[arg(long)]
        asset: PathBuf,
        /// Local full-ROM output used for emulator verification.
        #[arg(long)]
        output: PathBuf,
        /// Optional PNG preview after NES palette and resolution conversion.
        #[arg(long)]
        preview: Option<PathBuf>,
    },
    /// Extract the complete verified Japanese ending corpus as reviewable JSON.
    ExtractEndingCorpus {
        /// Exact supported Japanese ROM.
        rom: PathBuf,
        /// Local JSON output used for corpus review and translation planning.
        #[arg(long)]
        output: PathBuf,
    },
    /// Validate the review draft and plan independent 1 KiB Hangul font pages.
    PlanEndingTranslation {
        /// Exact supported Japanese ROM.
        rom: PathBuf,
        /// Committed Korean ending draft.
        #[arg(long)]
        translation: PathBuf,
        /// Local JSON plan used for review and font-page implementation.
        #[arg(long)]
        output: PathBuf,
    },
    /// Build a local full-ending PoC from the unreviewed Korean draft.
    BuildEndingDraftPoc {
        /// Exact supported Japanese ROM.
        rom: PathBuf,
        /// Committed, project-owner-approved Korean ending translation.
        #[arg(long)]
        translation: PathBuf,
        /// Local full-ROM output used for emulator verification.
        #[arg(long)]
        output: PathBuf,
    },
    /// Build one Japanese-ROM-based draft with Korean baked graphics and ending.
    BuildKoreanDraft {
        /// Exact supported Japanese ROM.
        rom: PathBuf,
        /// Committed RGBA PNG source for the Korean title graphic.
        #[arg(long)]
        title_asset: PathBuf,
        /// Committed, project-owner-approved baked-graphics translation.
        #[arg(long)]
        graphics_translation: PathBuf,
        /// Committed, project-owner-approved Korean ending translation.
        #[arg(long)]
        ending_translation: PathBuf,
        /// Local full-ROM output used for integrated emulator verification.
        #[arg(long)]
        output: PathBuf,
        /// Optional BPS1 distribution patch for the headerless PRG+CHR source.
        #[arg(long)]
        bps_output: Option<PathBuf>,
        /// Optional BPS1 compatibility patch for the source with an iNES header.
        #[arg(long)]
        bps_ines_header_output: Option<PathBuf>,
    },
    /// Render an exact 40x16 NES C00 font/size sweep without changing the production build.
    SweepRouletteLabel {
        /// Korean label to render.
        #[arg(long, default_value = "룰렛")]
        text: String,
        /// Candidate TTF/OTF path. Repeat for each font.
        #[arg(long, required = true)]
        font: Vec<PathBuf>,
        /// Candidate pixel sizes, comma-separated. Defaults to 10 through 16.
        #[arg(long, value_delimiter = ',')]
        size: Vec<f32>,
        /// Coverage threshold used to binarize each glyph.
        #[arg(long, default_value_t = 128)]
        threshold: u8,
        /// Extra pixels inserted between visible glyph bounds.
        #[arg(long, default_value_t = 3)]
        letter_spacing: usize,
        /// Percentage of glyph height using the upper fill before lower shading.
        #[arg(long, default_value_t = 60)]
        shade_split: u8,
        /// Local PNG contact sheet output.
        #[arg(long)]
        output: PathBuf,
    },
    /// Apply a BPS1 patch after strict source, patch, and target CRC validation.
    ApplyBps {
        /// Exact source file expected by the patch, including headerless sources.
        rom: PathBuf,
        /// BPS1 patch.
        #[arg(long)]
        patch: PathBuf,
        /// Reconstructed full-ROM output used for local verification.
        #[arg(long)]
        output: PathBuf,
    },
}
