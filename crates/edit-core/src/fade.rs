//! Fade in / fade out applied to a stream of big-endian PCM as it is read.
//!
//! Done in-process on the sample data (rather than as an ffmpeg filter) so the identical fade is
//! applied whichever way the audio leaves: AIFF writer, ffmpeg, or Apple's encoder.
//! The curve is a quarter sine, which keeps perceived loudness changing smoothly (equal power).

use std::io::{self, Read};

/// Fade lengths in frames. Zero means no fade at that end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fade {
    pub in_frames: u64,
    pub out_frames: u64,
}

impl Fade {
    pub fn from_secs(fade_in: f64, fade_out: f64, rate: f64) -> Fade {
        let frames = |s: f64| {
            if s.is_finite() && s > 0.0 {
                (s * rate).round() as u64
            } else {
                0
            }
        };
        Fade {
            in_frames: frames(fade_in),
            out_frames: frames(fade_out),
        }
    }

    pub fn is_none(&self) -> bool {
        self.in_frames == 0 && self.out_frames == 0
    }
}

/// Gain (0..=1) for `frame` of a `total`-frame stream.
pub fn gain(fade: Fade, frame: u64, total: u64) -> f64 {
    let curve = |t: f64| (t.clamp(0.0, 1.0) * std::f64::consts::FRAC_PI_2).sin();
    let mut g = 1.0;
    if frame < fade.in_frames {
        g *= curve(frame as f64 / fade.in_frames as f64);
    }
    let from_end = total.saturating_sub(1).saturating_sub(frame);
    if fade.out_frames > 0 && from_end < fade.out_frames {
        g *= curve(from_end as f64 / fade.out_frames as f64);
    }
    g
}

const CHUNK: usize = 64 * 1024;

/// Wraps a PCM reader and applies a [`Fade`]. Chunk boundaries never affect the result.
pub struct FadeReader<R> {
    inner: R,
    fade: Fade,
    bits: u16,
    channels: usize,
    align: usize,
    total_frames: u64,
    /// Index of the next frame to be processed.
    frame: u64,
    scratch: Vec<u8>,
    /// Bytes of an incomplete trailing frame carried to the next read.
    carry: Vec<u8>,
    out: Vec<u8>,
    out_pos: usize,
    done: bool,
}

impl<R: Read> FadeReader<R> {
    pub fn new(inner: R, fade: Fade, bits: u16, channels: u16, total_frames: u64) -> Self {
        let channels = usize::from(channels.max(1));
        FadeReader {
            inner,
            fade,
            bits,
            channels,
            align: channels * usize::from(bits.div_ceil(8)),
            total_frames,
            frame: 0,
            scratch: vec![0; CHUNK],
            carry: Vec::new(),
            out: Vec::new(),
            out_pos: 0,
            done: false,
        }
    }

    fn apply(&self, block: &mut [u8], first_frame: u64) {
        let width = usize::from(self.bits.div_ceil(8));
        let frames = (block.len() / self.align) as u64;
        let last = first_frame + frames;
        let in_zone = first_frame < self.fade.in_frames;
        let out_zone = self.fade.out_frames > 0
            && last > self.total_frames.saturating_sub(self.fade.out_frames);
        if !in_zone && !out_zone {
            return;
        }
        let (min, max) = match self.bits {
            8 => (-128i64, 127i64),
            16 => (-32768, 32767),
            24 => (-8_388_608, 8_388_607),
            _ => (-2_147_483_648, 2_147_483_647),
        };
        for (i, frame) in block.chunks_exact_mut(self.align).enumerate() {
            let g = gain(self.fade, first_frame + i as u64, self.total_frames);
            if g >= 1.0 {
                continue;
            }
            for ch in 0..self.channels {
                let s = &mut frame[ch * width..(ch + 1) * width];
                // Sign-extend the big-endian sample, scale it, and write it back.
                let mut v: i64 = if s[0] & 0x80 != 0 { -1 } else { 0 };
                for &b in s.iter() {
                    v = (v << 8) | i64::from(b);
                }
                let scaled = ((v as f64) * g).round() as i64;
                let scaled = scaled.clamp(min, max);
                for (k, byte) in s.iter_mut().enumerate() {
                    *byte = (scaled >> (8 * (width - 1 - k))) as u8;
                }
            }
        }
    }

    /// Fill `out` with the next processed block. Leaves it empty at end of stream.
    fn refill(&mut self) -> io::Result<()> {
        self.out.clear();
        self.out_pos = 0;
        while self.out.is_empty() && !self.done {
            let n = self.inner.read(&mut self.scratch)?;
            if n == 0 {
                self.done = true;
                // A trailing partial frame cannot be faded meaningfully; pass it through.
                self.out.append(&mut self.carry);
                break;
            }
            self.carry.extend_from_slice(&self.scratch[..n]);
            let whole = self.carry.len() - self.carry.len() % self.align;
            if whole == 0 {
                continue;
            }
            let mut block: Vec<u8> = self.carry.drain(..whole).collect();
            let first = self.frame;
            self.apply(&mut block, first);
            self.frame += (whole / self.align) as u64;
            self.out = block;
        }
        Ok(())
    }
}

impl<R: Read> Read for FadeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.out_pos >= self.out.len() {
            self.refill()?;
            if self.out.is_empty() {
                return Ok(0);
            }
        }
        let n = buf.len().min(self.out.len() - self.out_pos);
        buf[..n].copy_from_slice(&self.out[self.out_pos..self.out_pos + n]);
        self.out_pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that returns data in awkward, caller-chosen piece sizes.
    struct Chunky<'a> {
        data: &'a [u8],
        sizes: Vec<usize>,
        i: usize,
    }
    impl Read for Chunky<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let want = self.sizes[self.i % self.sizes.len()]
                .min(buf.len())
                .min(self.data.len());
            self.i += 1;
            buf[..want].copy_from_slice(&self.data[..want]);
            self.data = &self.data[want..];
            Ok(want)
        }
    }

    fn s16(frames: &[(i16, i16)]) -> Vec<u8> {
        frames
            .iter()
            .flat_map(|&(l, r)| [l.to_be_bytes(), r.to_be_bytes()].concat())
            .collect()
    }

    fn faded(data: &[u8], fade: Fade, bits: u16, channels: u16, sizes: Vec<usize>) -> Vec<u8> {
        let total = (data.len() / (usize::from(channels) * usize::from(bits / 8))) as u64;
        let mut r = FadeReader::new(Chunky { data, sizes, i: 0 }, fade, bits, channels, total);
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        out
    }

    fn decode16(bytes: &[u8]) -> Vec<i16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| i16::from_be_bytes(c))
            .collect()
    }

    #[test]
    fn the_curve_starts_silent_ends_full_and_is_equal_power_in_the_middle() {
        let f = Fade {
            in_frames: 1000,
            out_frames: 0,
        };
        assert_eq!(gain(f, 0, 100_000), 0.0);
        assert!((gain(f, 500, 100_000) - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        assert_eq!(gain(f, 1000, 100_000), 1.0);
        assert_eq!(gain(f, 99_999, 100_000), 1.0);
        let o = Fade {
            in_frames: 0,
            out_frames: 1000,
        };
        assert_eq!(gain(o, 99_999, 100_000), 0.0);
        assert_eq!(gain(o, 0, 100_000), 1.0);
        assert!((gain(o, 99_499, 100_000) - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-3);
    }

    #[test]
    fn gain_never_rises_during_a_fade_out_or_falls_during_a_fade_in() {
        let f = Fade {
            in_frames: 500,
            out_frames: 500,
        };
        let g: Vec<f64> = (0..5000).map(|i| gain(f, i, 5000)).collect();
        assert!(g[..500].windows(2).all(|w| w[0] <= w[1]));
        assert!(g[4500..].windows(2).all(|w| w[0] >= w[1]));
        assert!(g[500..4500].iter().all(|&x| x == 1.0));
        assert!(g.iter().all(|&x| (0.0..=1.0).contains(&x)));
    }

    #[test]
    fn no_fade_passes_the_bytes_through_unchanged() {
        let data = s16(&(0..1000)
            .map(|i| (i as i16 * 30, -(i as i16) * 20))
            .collect::<Vec<_>>());
        assert_eq!(faded(&data, Fade::default(), 16, 2, vec![4096]), data);
    }

    #[test]
    fn fade_in_starts_at_silence_and_leaves_the_rest_alone() {
        let data = s16(&vec![(10000, -10000); 2000]);
        let out = decode16(&faded(
            &data,
            Fade {
                in_frames: 500,
                out_frames: 0,
            },
            16,
            2,
            vec![4096],
        ));
        assert_eq!((out[0], out[1]), (0, 0));
        assert!(out[2] > 0 && out[3] < 0, "the sign is preserved");
        let left: Vec<i16> = out.chunks(2).map(|f| f[0]).collect();
        assert!(
            left[..500].windows(2).all(|w| w[0] <= w[1]),
            "rises monotonically"
        );
        assert!(
            left[500..].iter().all(|&v| v == 10000),
            "untouched after the fade"
        );
    }

    #[test]
    fn fade_out_ends_at_silence() {
        let data = s16(&vec![(10000, 10000); 2000]);
        let out = decode16(&faded(
            &data,
            Fade {
                in_frames: 0,
                out_frames: 500,
            },
            16,
            2,
            vec![4096],
        ));
        let left: Vec<i16> = out.chunks(2).map(|f| f[0]).collect();
        assert!(left[..1500].iter().all(|&v| v == 10000));
        assert_eq!(*left.last().unwrap(), 0);
        assert!(
            left[1500..].windows(2).all(|w| w[0] >= w[1]),
            "falls monotonically"
        );
    }

    #[test]
    fn the_read_sizes_do_not_change_the_result() {
        let data = s16(&(0..5000)
            .map(|i| {
                (
                    (i * 7 % 20000) as i16 - 10000,
                    (i * 13 % 20000) as i16 - 10000,
                )
            })
            .collect::<Vec<_>>());
        let fade = Fade {
            in_frames: 1200,
            out_frames: 800,
        };
        let reference = faded(&data, fade, 16, 2, vec![1 << 20]);
        for sizes in [
            vec![1],
            vec![3],
            vec![7, 2, 5],
            vec![4],
            vec![4095],
            vec![1, 100000],
        ] {
            assert_eq!(
                faded(&data, fade, 16, 2, sizes.clone()),
                reference,
                "sizes {sizes:?}"
            );
        }
    }

    #[test]
    fn overlapping_fades_multiply_on_a_short_clip() {
        let data = s16(&vec![(20000, 20000); 100]);
        let out = decode16(&faded(
            &data,
            Fade {
                in_frames: 80,
                out_frames: 80,
            },
            16,
            2,
            vec![64],
        ));
        let left: Vec<i16> = out.chunks(2).map(|f| f[0]).collect();
        assert_eq!(left[0], 0);
        assert_eq!(left[99], 0);
        assert!(left.iter().all(|&v| (0..=20000).contains(&v)));
        assert!(
            left[50] < 20000,
            "the overlap keeps even the middle below full scale"
        );
    }

    #[test]
    fn a_fade_longer_than_the_audio_just_covers_all_of_it() {
        let data = s16(&[(20000, 20000); 50]);
        let out = decode16(&faded(
            &data,
            Fade {
                in_frames: 100_000,
                out_frames: 0,
            },
            16,
            2,
            vec![64],
        ));
        assert_eq!(out[0], 0);
        assert!(out.iter().all(|&v| v < 20000));
    }

    #[test]
    fn works_for_24_bit_mono() {
        // Full-scale 24-bit samples fading in.
        let frames = 1000usize;
        let data: Vec<u8> = (0..frames).flat_map(|_| [0x40, 0x00, 0x00]).collect(); // +4194304
        let out = faded(
            &data,
            Fade {
                in_frames: 400,
                out_frames: 0,
            },
            24,
            1,
            vec![997],
        );
        let v = |i: usize| {
            ((i32::from(out[i * 3] as i8) << 16)
                | (i32::from(out[i * 3 + 1]) << 8)
                | i32::from(out[i * 3 + 2])) as i64
        };
        assert_eq!(v(0), 0);
        assert!((v(200) as f64 / 4_194_304.0 - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01);
        assert_eq!(v(400), 4_194_304);
        assert_eq!(v(999), 4_194_304);
    }

    #[test]
    fn works_for_8_and_32_bit_and_negative_samples_round_symmetrically() {
        let eight: Vec<u8> = vec![0x80u8; 100]; // -128
        let out = faded(
            &eight,
            Fade {
                in_frames: 100,
                out_frames: 0,
            },
            8,
            1,
            vec![7],
        );
        assert_eq!(out[0] as i8, 0);
        assert!((out[50] as i8) < 0 && (out[50] as i8) > -128);
        assert_eq!(
            out[99] as i8,
            ((-128.0f64)
                * gain(
                    Fade {
                        in_frames: 100,
                        out_frames: 0
                    },
                    99,
                    100
                ))
            .round() as i8
        );

        let pos: Vec<u8> = [0x40u8, 0, 0, 0].repeat(64); // +2^30
        let neg: Vec<u8> = [0xC0u8, 0, 0, 0].repeat(64); // -2^30
        let f = Fade {
            in_frames: 64,
            out_frames: 0,
        };
        let (p, n) = (
            faded(&pos, f, 32, 1, vec![5]),
            faded(&neg, f, 32, 1, vec![5]),
        );
        for i in 0..64 {
            let (pv, nv) = (
                i32::from_be_bytes(p[i * 4..i * 4 + 4].try_into().unwrap()),
                i32::from_be_bytes(n[i * 4..i * 4 + 4].try_into().unwrap()),
            );
            assert_eq!(pv, -nv, "frame {i}");
        }
    }

    #[test]
    fn a_trailing_partial_frame_is_passed_through_not_dropped() {
        let mut data = s16(&[(1000, 1000); 10]);
        data.extend_from_slice(&[0xAB, 0xCD, 0xEF]);
        let out = faded(
            &data,
            Fade {
                in_frames: 5,
                out_frames: 0,
            },
            16,
            2,
            vec![6],
        );
        assert_eq!(out.len(), data.len());
        assert_eq!(&out[out.len() - 3..], &[0xAB, 0xCD, 0xEF]);
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert!(faded(
            &[],
            Fade {
                in_frames: 10,
                out_frames: 10
            },
            16,
            2,
            vec![8]
        )
        .is_empty());
    }

    #[test]
    fn from_secs_converts_and_ignores_nonsense() {
        assert_eq!(
            Fade::from_secs(2.0, 0.5, 44100.0),
            Fade {
                in_frames: 88200,
                out_frames: 22050
            }
        );
        assert!(Fade::from_secs(0.0, -3.0, 44100.0).is_none());
        assert!(Fade::from_secs(f64::NAN, f64::INFINITY, 44100.0).is_none());
    }
}
