//! Canonical 54-byte AIFF header (FORM + COMM + SSND with zero offset/blockSize).

use crate::{ext80, Format};

pub const HEADER_LEN: u64 = 54;

/// Header for `audio_bytes` of PCM, assuming nothing follows the audio. Odd-length
/// audio gets a pad byte, which the caller must append.
pub fn build(format: &Format, frames: u32, audio_bytes: u64) -> [u8; HEADER_LEN as usize] {
    let pad = audio_bytes & 1;
    let form_size = (HEADER_LEN - 8 + audio_bytes + pad) as u32;
    let ssnd_size = (8 + audio_bytes) as u32;
    let mut h = [0u8; HEADER_LEN as usize];
    h[0..4].copy_from_slice(b"FORM");
    h[4..8].copy_from_slice(&form_size.to_be_bytes());
    h[8..12].copy_from_slice(b"AIFF");
    h[12..16].copy_from_slice(b"COMM");
    h[16..20].copy_from_slice(&18u32.to_be_bytes());
    h[20..22].copy_from_slice(&format.channels.to_be_bytes());
    h[22..26].copy_from_slice(&frames.to_be_bytes());
    h[26..28].copy_from_slice(&format.bits.to_be_bytes());
    h[28..38].copy_from_slice(&ext80::encode(format.sample_rate));
    h[38..42].copy_from_slice(b"SSND");
    h[42..46].copy_from_slice(&ssnd_size.to_be_bytes());
    // offset and blockSize stay zero
    h
}
