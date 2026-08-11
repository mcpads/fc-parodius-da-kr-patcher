pub mod bps;
pub mod build;
pub mod cli;
pub mod ending;
pub mod font;
pub mod graphics_translation;
pub mod rom;
pub mod tracked;
pub mod translation;

use sha1::{Digest, Sha1};

pub fn sha1_hex(data: &[u8]) -> String {
    format!("{:x}", Sha1::digest(data))
}
