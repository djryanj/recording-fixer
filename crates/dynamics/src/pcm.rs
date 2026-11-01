//! Big-endian integer PCM (as in AIFF) to and from f32, with dither on the way back.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PcmFormat {
    pub bits: u16,
    pub channels: u16,
    pub rate: f64,
}

impl PcmFormat {
    /// Bytes per sample as stored.
    pub fn width(&self) -> usize {
        usize::from(self.bits.div_ceil(8)).clamp(1, 4)
    }

    pub fn frame_bytes(&self) -> usize {
        self.width() * usize::from(self.channels)
    }

    /// Value of one step of the integer scale, as a fraction of full scale.
    fn full_scale(&self) -> f32 {
        (1u64 << (self.width() * 8 - 1)) as f32
    }
}

/// Append the samples of whole frames in `bytes` to `out`, scaled to +-1.
pub fn decode(fmt: PcmFormat, bytes: &[u8], out: &mut Vec<f32>) {
    let w = fmt.width();
    let inv = 1.0 / fmt.full_scale();
    out.reserve(bytes.len() / w);
    for s in bytes.chunks_exact(w) {
        let mut v: i32 = if s[0] & 0x80 != 0 { -1 } else { 0 };
        for &b in s {
            v = (v << 8) | i32::from(b);
        }
        out.push(v as f32 * inv);
    }
}

/// Small deterministic generator, so a given input always dithers the same way.
struct Rng(u64);

impl Rng {
    /// Uniform in [0, 1).
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let r = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (r >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Rounds samples back to integers. At 16 bits and below, TPDF dither of one step is added first, so
/// the rounding error becomes plain low-level noise instead of distortion that follows the music.
/// Samples that are already on the integer grid (the signal was not changed) pass through exactly.
pub struct Quantizer {
    fmt: PcmFormat,
    dither: bool,
    rng: Rng,
}

impl Quantizer {
    pub fn new(fmt: PcmFormat) -> Quantizer {
        Quantizer {
            fmt,
            dither: fmt.bits <= 16,
            rng: Rng(0x9E37_79B9_7F4A_7C15),
        }
    }

    /// Quantise `samples` and append them as big-endian bytes. Also writes back what was actually
    /// stored, as f32, into `stored` (for measuring the result).
    pub fn encode(&mut self, samples: &[f32], out: &mut Vec<u8>, stored: &mut Vec<f32>) {
        let w = self.fmt.width();
        let scale = self.fmt.full_scale();
        let (lo, hi) = (-scale, scale - 1.0);
        out.reserve(samples.len() * w);
        for &x in samples {
            let v = x * scale;
            let rounded = v.round();
            let q = if self.dither && (v - rounded).abs() > 1e-3 {
                (v + self.rng.next() - self.rng.next()).round()
            } else {
                rounded
            };
            let q = q.clamp(lo, hi);
            let i = q as i32;
            out.extend_from_slice(&i.to_be_bytes()[4 - w..]);
            stored.push(q / scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(bits: u16) -> PcmFormat {
        PcmFormat {
            bits,
            channels: 2,
            rate: 44_100.0,
        }
    }

    #[test]
    fn decoding_then_encoding_untouched_audio_is_bit_exact() {
        for bits in [16u16, 24] {
            let w = usize::from(bits / 8);
            let bytes: Vec<u8> = (0..4000u32)
                .map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8)
                .collect();
            let bytes = &bytes[..bytes.len() / (2 * w) * (2 * w)];
            let mut f = Vec::new();
            decode(fmt(bits), bytes, &mut f);
            let (mut out, mut stored) = (Vec::new(), Vec::new());
            Quantizer::new(fmt(bits)).encode(&f, &mut out, &mut stored);
            assert_eq!(out, bytes, "{bits}-bit");
        }
    }

    #[test]
    fn extremes_and_signs_decode_correctly() {
        let mut f = Vec::new();
        decode(fmt(16), &[0x80, 0x00, 0x7F, 0xFF], &mut f);
        assert_eq!(f, vec![-1.0, 32767.0 / 32768.0]);
        f.clear();
        decode(fmt(24), &[0x80, 0x00, 0x00, 0x00, 0x00, 0x01], &mut f);
        assert_eq!(f, vec![-1.0, 1.0 / 8_388_608.0]);
    }

    #[test]
    fn clipping_is_clamped_not_wrapped() {
        let (mut out, mut stored) = (Vec::new(), Vec::new());
        Quantizer::new(fmt(16)).encode(&[1.5, -1.5], &mut out, &mut stored);
        assert_eq!(out, vec![0x7F, 0xFF, 0x80, 0x00]);
    }

    #[test]
    fn dither_turns_a_sub_lsb_signal_into_noise_that_averages_out() {
        // A constant half-step value: without dither it would always round the same way.
        let x = vec![0.4 / 32768.0; 200_000];
        let (mut out, mut stored) = (Vec::new(), Vec::new());
        Quantizer::new(fmt(16)).encode(&x, &mut out, &mut stored);
        let mean: f64 =
            stored.iter().map(|&v| f64::from(v) * 32768.0).sum::<f64>() / stored.len() as f64;
        assert!((mean - 0.4).abs() < 0.02, "{mean}");
    }

    #[test]
    fn no_dither_at_24_bits() {
        let (mut out, mut stored) = (Vec::new(), Vec::new());
        Quantizer::new(fmt(24)).encode(&[0.25 + 0.3 / 8_388_608.0; 1000], &mut out, &mut stored);
        assert!(out.chunks(3).all(|c| c == &out[..3]));
    }
}
