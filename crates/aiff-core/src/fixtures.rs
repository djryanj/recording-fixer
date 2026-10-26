//! Synthetic AIFF files that mimic what Serato writes, for tests.

use crate::{header, Format};

#[derive(Clone, Copy)]
pub struct Spec {
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub frames: u32,
}

impl Spec {
    pub const fn cd(frames: u32) -> Self {
        Spec {
            rate: 44100,
            channels: 2,
            bits: 16,
            frames,
        }
    }
    pub fn format(&self) -> Format {
        Format {
            channels: self.channels,
            bits: self.bits,
            sample_rate: f64::from(self.rate),
        }
    }
}

/// Deterministic PCM so tests can tell frames apart. `seed` shifts the pattern per file.
pub fn pcm(spec: Spec, seed: u32) -> Vec<u8> {
    let width = usize::from(spec.bits / 8);
    let mut out = Vec::with_capacity(spec.frames as usize * usize::from(spec.channels) * width);
    for i in 0..spec.frames {
        for c in 0..u32::from(spec.channels) {
            let v = ((i
                .wrapping_mul(37)
                .wrapping_add(c * 1000)
                .wrapping_add(seed * 7919))
                % 20000) as i32
                - 10000;
            let shifted = if spec.bits >= 16 {
                i64::from(v) << (spec.bits - 16)
            } else {
                i64::from(v) >> (16 - spec.bits)
            };
            out.extend_from_slice(&shifted.to_be_bytes()[8 - width..]);
        }
    }
    out
}

/// Serato's ID3 trailer: an `ID3 ` chunk holding TIT2 and a "Recorded ..." COMM comment.
pub fn id3_chunk(recorded: &str) -> Vec<u8> {
    fn frame(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut f = id.to_vec();
        f.extend_from_slice(&(body.len() as u32).to_be_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(body);
        f
    }
    let mut tit2 = vec![0u8];
    tit2.extend_from_slice(b"Serato Recording");
    let mut comm = vec![0u8];
    comm.extend_from_slice(b"eng");
    comm.push(0);
    comm.extend_from_slice(recorded.as_bytes());
    let mut frames = frame(b"TIT2", &tit2);
    frames.extend(frame(b"COMM", &comm));
    let n = frames.len() as u32;
    let synchsafe = [
        (n >> 21) as u8 & 0x7f,
        (n >> 14) as u8 & 0x7f,
        (n >> 7) as u8 & 0x7f,
        n as u8 & 0x7f,
    ];
    let mut tag = b"ID3\x03\x00\x00".to_vec();
    tag.extend_from_slice(&synchsafe);
    tag.extend(frames);
    let mut chunk = b"ID3 ".to_vec();
    chunk.extend_from_slice(&(tag.len() as u32).to_be_bytes());
    chunk.extend(tag);
    if chunk.len() % 2 == 1 {
        chunk.push(0);
    }
    chunk
}

/// A cleanly closed recording: consistent sizes plus the ID3 trailer.
pub fn healthy(spec: Spec, seed: u32) -> Vec<u8> {
    let audio = pcm(spec, seed);
    let mut f = header::build(&spec.format(), spec.frames, audio.len() as u64).to_vec();
    f.extend_from_slice(&audio);
    let trailer = id3_chunk("Recorded 2025-10-04 10:31:51");
    let form = (f.len() - 8 + trailer.len()) as u32;
    f[4..8].copy_from_slice(&form.to_be_bytes());
    f.extend(trailer);
    f
}

/// An unclosed recording: FORM size, frame count and SSND size are all zero,
/// and there is no trailer. `extra` appends a partial frame of junk bytes.
pub fn broken(spec: Spec, seed: u32, extra: usize) -> Vec<u8> {
    let audio = pcm(spec, seed);
    let mut f = header::build(&spec.format(), spec.frames, audio.len() as u64).to_vec();
    f[4..8].fill(0);
    f[22..26].fill(0);
    f[42..46].fill(0);
    f.extend_from_slice(&audio);
    f.extend(std::iter::repeat_n(0xAB, extra));
    f
}
