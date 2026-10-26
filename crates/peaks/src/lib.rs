//! Waveform peaks: a multi-resolution min/max pyramid built in one streaming pass.
//!
//! Each bucket stores the minimum and maximum of the most significant byte of every sample
//! (for big-endian signed PCM that byte is the sample's value at 8-bit precision, which is
//! plenty for drawing). A 3 hour 44.1 kHz recording needs about 4 MB at the finest level.

pub mod cache;

use aiff_core::Format;
use std::io::Read;
use std::path::Path;

/// Frames per bucket at the finest level.
pub const BASE: u64 = 256;
/// Each level is this many times coarser than the previous one.
pub const FACTOR: u64 = 4;
/// Stop adding levels once one is this small.
const MIN_TOP_BUCKETS: usize = 2048;
/// Frames read per chunk (bounds memory regardless of file size).
const CHUNK_FRAMES: u64 = 1 << 16;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Aiff(#[from] aiff_core::Error),
    #[error("corrupt peak cache")]
    Corrupt,
    #[error("cancelled")]
    Cancelled,
    #[error("too many frames requested at once")]
    TooLarge,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq)]
pub struct Peaks {
    pub frames: u64,
    /// `levels[k]` holds interleaved `[min, max]` pairs; one bucket covers `BASE * FACTOR^k` frames.
    pub levels: Vec<Vec<i8>>,
}

pub fn bucket_frames(level: usize) -> u64 {
    BASE * FACTOR.pow(level as u32)
}

fn merge(prev: &[i8]) -> Vec<i8> {
    prev.chunks(2 * FACTOR as usize)
        .flat_map(|group| {
            let mut min = i8::MAX;
            let mut max = i8::MIN;
            for pair in group.as_chunks::<2>().0 {
                min = min.min(pair[0]);
                max = max.max(pair[1]);
            }
            [min, max]
        })
        .collect()
}

fn pyramid(base: Vec<i8>, frames: u64) -> Peaks {
    let mut levels = vec![base];
    while levels.last().is_some_and(|l| l.len() / 2 > MIN_TOP_BUCKETS) {
        let next = merge(levels.last().unwrap());
        levels.push(next);
    }
    Peaks { frames, levels }
}

/// Build peaks from `frames` frames of big-endian PCM read from `reader`.
/// `progress(done_frames, total_frames)` returns false to cancel.
pub fn build<R: Read>(
    reader: &mut R,
    format: &Format,
    frames: u64,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<Peaks> {
    build_with(reader, format, frames, progress, &mut |_| {})
}

/// Like [`build`], but also hands over the finest-level buckets as soon as each chunk of audio has
/// produced them (`[min, max]` pairs, in order, each bucket `BASE` frames), so a caller can draw the
/// waveform while it is still being built.
pub fn build_with<R: Read>(
    reader: &mut R,
    format: &Format,
    frames: u64,
    progress: &mut dyn FnMut(u64, u64) -> bool,
    on_buckets: &mut dyn FnMut(&[i8]),
) -> Result<Peaks> {
    let align = format.block_align() as usize;
    let sample_bytes = format.bytes_per_sample() as usize;
    let channels = usize::from(format.channels);
    let mut buf = vec![0u8; CHUNK_FRAMES as usize * align];
    let mut base: Vec<i8> = Vec::with_capacity((frames / BASE + 1) as usize * 2);
    let (mut min, mut max, mut in_bucket) = (i8::MAX, i8::MIN, 0u64);
    let mut done = 0u64;
    let mut announced = 0usize;
    while done < frames {
        let n = (frames - done).min(CHUNK_FRAMES) as usize;
        reader.read_exact(&mut buf[..n * align])?;
        let mut f = 0usize;
        while f < n {
            let take = ((BASE - in_bucket) as usize).min(n - f);
            for frame in buf[f * align..(f + take) * align].chunks_exact(align) {
                for c in 0..channels {
                    let v = frame[c * sample_bytes] as i8;
                    min = min.min(v);
                    max = max.max(v);
                }
            }
            in_bucket += take as u64;
            f += take;
            if in_bucket == BASE {
                base.extend([min, max]);
                (min, max, in_bucket) = (i8::MAX, i8::MIN, 0);
            }
        }
        done += n as u64;
        on_buckets(&base[announced..]);
        announced = base.len();
        if !progress(done, frames) {
            return Err(Error::Cancelled);
        }
    }
    if in_bucket > 0 {
        base.extend([min, max]);
        on_buckets(&base[announced..]);
    }
    Ok(pyramid(base, frames))
}

/// Build peaks for a whole AIFF file (healthy or broken) by streaming its audio.
pub fn build_for_file(path: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<Peaks> {
    build_for_file_with(path, progress, &mut |_| {})
}

/// [`build_for_file`] with the finer-level buckets handed over as they are made (see [`build_with`]).
pub fn build_for_file_with(
    path: &Path,
    progress: &mut dyn FnMut(u64, u64) -> bool,
    on_buckets: &mut dyn FnMut(&[i8]),
) -> Result<Peaks> {
    use std::io::{BufReader, Seek, SeekFrom};
    let d = aiff_core::diagnose_path(path)?;
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(d.audio_start))?;
    build_with(
        &mut BufReader::with_capacity(1 << 20, f),
        &d.format,
        d.frames,
        progress,
        on_buckets,
    )
}

/// Most frames `read_samples` will return in one call (bounds memory whatever the UI asks).
pub const MAX_SAMPLE_FRAMES: u64 = 1 << 18;

/// The `[min, max]` of every frame across its channels, as 16-bit values (interleaved), from raw
/// big-endian PCM. Samples wider than 16 bits keep their top 16 bits; 8-bit ones are scaled up.
pub fn frame_extremes(pcm: &[u8], format: &Format) -> Vec<i16> {
    let align = format.block_align() as usize;
    let sb = format.bytes_per_sample() as usize;
    let channels = usize::from(format.channels);
    let mut out = Vec::with_capacity(pcm.len() / align * 2);
    for frame in pcm.chunks_exact(align) {
        let (mut min, mut max) = (i16::MAX, i16::MIN);
        for c in 0..channels {
            let b = &frame[c * sb..];
            let v = if sb == 1 {
                i16::from(b[0] as i8) << 8
            } else {
                i16::from_be_bytes([b[0], b[1]])
            };
            min = min.min(v);
            max = max.max(v);
        }
        out.extend([min, max]);
    }
    out
}

/// Exact `[min, max]` 16-bit values for each frame in `[start, end)` of an AIFF file (healthy or
/// broken), for zooming in far enough to see individual samples. Reads only that range.
pub fn read_samples(path: &Path, start: u64, end: u64) -> Result<Vec<i16>> {
    use std::io::{Seek, SeekFrom};
    if end.saturating_sub(start) > MAX_SAMPLE_FRAMES {
        return Err(Error::TooLarge);
    }
    let d = aiff_core::diagnose_path(path)?;
    let end = end.min(d.frames);
    if start >= end {
        return Ok(Vec::new());
    }
    let align = d.format.block_align();
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(d.audio_start + start * align))?;
    let mut buf = vec![0u8; ((end - start) * align) as usize];
    f.read_exact(&mut buf)?;
    Ok(frame_extremes(&buf, &d.format))
}

impl Peaks {
    /// What a build has produced so far: only the finest level, covering the first `frames` frames.
    /// Good enough to draw from while the rest is still being made.
    pub fn partial(base: Vec<i8>, frames: u64) -> Peaks {
        Peaks {
            frames,
            levels: vec![base],
        }
    }

    /// Reduce the frame range `[start, end)` to `width` columns of `[min, max]` (interleaved).
    /// Uses the coarsest level that still has at least one bucket per column.
    pub fn columns(&self, start: u64, end: u64, width: usize) -> Vec<i8> {
        let end = end.min(self.frames);
        if width == 0 || start >= end {
            return Vec::new();
        }
        let span = end - start;
        let per_col = (span as f64 / width as f64).max(1.0);
        let level = (0..self.levels.len())
            .rev()
            .find(|&l| bucket_frames(l) as f64 <= per_col)
            .unwrap_or(0);
        let (bucket, data) = (bucket_frames(level), &self.levels[level]);
        let buckets = data.len() / 2;
        if buckets == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(width * 2);
        for x in 0..width {
            let a = start + (span as u128 * x as u128 / width as u128) as u64;
            let b = (start + (span as u128 * (x as u128 + 1) / width as u128) as u64).max(a + 1);
            let first = ((a / bucket) as usize).min(buckets - 1);
            let last = (((b - 1) / bucket) as usize).clamp(first, buckets - 1);
            let (mut min, mut max) = (i8::MAX, i8::MIN);
            for i in first..=last {
                min = min.min(data[i * 2]);
                max = max.max(data[i * 2 + 1]);
            }
            out.extend([min, max]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiff_core::fixtures::{self, Spec};

    fn no_progress() -> impl FnMut(u64, u64) -> bool {
        |_, _| true
    }

    /// Reference implementation: straightforward per-bucket scan of 16-bit stereo PCM.
    fn naive(pcm: &[u8], align: usize, sample_bytes: usize, channels: usize) -> Vec<i8> {
        pcm.chunks(align * BASE as usize)
            .flat_map(|bucket| {
                let (mut min, mut max) = (i8::MAX, i8::MIN);
                for frame in bucket.chunks_exact(align) {
                    for c in 0..channels {
                        let v = frame[c * sample_bytes] as i8;
                        min = min.min(v);
                        max = max.max(v);
                    }
                }
                [min, max]
            })
            .collect()
    }

    #[test]
    fn exact_samples_keep_the_full_16_bit_value_of_every_frame() {
        // Stereo frames: (1000, -3000), (-32768, 32767), (5, 6).
        let mut pcm = Vec::new();
        for (l, r) in [(1000i16, -3000i16), (i16::MIN, i16::MAX), (5, 6)] {
            pcm.extend(l.to_be_bytes());
            pcm.extend(r.to_be_bytes());
        }
        let out = frame_extremes(&pcm, &Spec::cd(0).format());
        // [min, max] per frame across the channels; nothing is rounded to 8 bits.
        assert_eq!(out, vec![-3000, 1000, i16::MIN, i16::MAX, 5, 6]);
    }

    #[test]
    fn exact_samples_scale_8_and_24_bit_to_16_bit() {
        let mono8 = Spec {
            rate: 8000,
            channels: 1,
            bits: 8,
            frames: 0,
        };
        assert_eq!(
            frame_extremes(&[0x7f, 0x80, 0x01], &mono8.format()),
            vec![0x7f00, 0x7f00, -0x8000, -0x8000, 0x0100, 0x0100]
        );
        let mono24 = Spec { bits: 24, ..mono8 };
        // The top 16 of 24 bits.
        assert_eq!(
            frame_extremes(&[0x12, 0x34, 0x56], &mono24.format()),
            vec![0x1234, 0x1234]
        );
    }

    #[test]
    fn read_samples_from_a_file_honours_the_header_and_clamps_the_range() {
        let spec = Spec::cd(1000);
        let pcm = fixtures::pcm(spec, 5);
        let dir = tempfile::tempdir().unwrap();
        let healthy = dir.path().join("h.aif");
        let broken = dir.path().join("b.aif");
        std::fs::write(&healthy, fixtures::healthy(spec, 5)).unwrap();
        std::fs::write(&broken, fixtures::broken(spec, 5, 0)).unwrap();
        for path in [&healthy, &broken] {
            let s = read_samples(path, 10, 14).unwrap();
            assert_eq!(
                s,
                frame_extremes(&pcm[10 * 4..14 * 4], &spec.format()),
                "{path:?}"
            );
            // Past the end is clamped, and an empty or reversed range is empty. The ID3 trailer is never audio.
            assert_eq!(read_samples(path, 998, 5000).unwrap().len(), 2 * 2);
            assert!(read_samples(path, 5, 5).unwrap().is_empty());
            assert!(read_samples(path, 2000, 3000).unwrap().is_empty());
        }
    }

    #[test]
    fn read_samples_refuses_a_huge_request_instead_of_allocating_it() {
        let spec = Spec::cd(10);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.aif");
        std::fs::write(&p, fixtures::healthy(spec, 1)).unwrap();
        assert!(matches!(
            read_samples(&p, 0, MAX_SAMPLE_FRAMES + 1),
            Err(Error::TooLarge)
        ));
    }

    #[test]
    fn level_zero_matches_a_naive_scan_including_the_partial_last_bucket() {
        let spec = Spec::cd(1000);
        let pcm = fixtures::pcm(spec, 3);
        let p = build(&mut &pcm[..], &spec.format(), 1000, &mut no_progress()).unwrap();
        assert_eq!(p.levels[0], naive(&pcm, 4, 2, 2));
        assert_eq!(p.levels[0].len(), 2 * 4); // ceil(1000 / 256) buckets
        assert_eq!(p.frames, 1000);
    }

    #[test]
    fn works_for_mono_24_bit() {
        let spec = Spec {
            rate: 48000,
            channels: 1,
            bits: 24,
            frames: 700,
        };
        let pcm = fixtures::pcm(spec, 1);
        let p = build(&mut &pcm[..], &spec.format(), 700, &mut no_progress()).unwrap();
        assert_eq!(p.levels[0], naive(&pcm, 3, 3, 1));
    }

    #[test]
    fn extremes_are_captured_per_bucket() {
        // 256 quiet frames, then one full-scale spike inside the second bucket.
        let mut pcm = vec![0u8; 512 * 4];
        pcm[(300 * 4)..(300 * 4 + 2)].copy_from_slice(&i16::MAX.to_be_bytes());
        pcm[(301 * 4)..(301 * 4 + 2)].copy_from_slice(&i16::MIN.to_be_bytes());
        let p = build(
            &mut &pcm[..],
            &Spec::cd(0).format(),
            512,
            &mut no_progress(),
        )
        .unwrap();
        assert_eq!(p.levels[0], vec![0, 0, -128, 127]);
    }

    #[test]
    fn buckets_are_handed_over_as_they_are_made_and_add_up_to_the_result() {
        let spec = Spec {
            rate: 8000,
            channels: 2,
            bits: 16,
            frames: (CHUNK_FRAMES * 3 + 1000) as u32,
        };
        let pcm = fixtures::pcm(spec, 5);
        let (mut got, mut calls, mut seen_during) = (Vec::new(), 0, Vec::new());
        let p = build_with(
            &mut &pcm[..],
            &spec.format(),
            u64::from(spec.frames),
            &mut |done, _| {
                seen_during.push(done);
                true
            },
            &mut |new| {
                calls += 1;
                got.extend_from_slice(new);
            },
        )
        .unwrap();
        assert_eq!(
            got, p.levels[0],
            "everything handed over, in order, nothing twice"
        );
        assert!(
            calls >= 4,
            "one hand-over per chunk, not one at the end ({calls})"
        );
        assert_eq!(seen_during.len(), 4);
    }

    #[test]
    fn a_partial_build_can_already_be_drawn() {
        let spec = Spec {
            rate: 8000,
            channels: 1,
            bits: 8,
            frames: (CHUNK_FRAMES * 4) as u32,
        };
        let pcm = fixtures::pcm(spec, 6);
        let full = build(
            &mut &pcm[..],
            &spec.format(),
            u64::from(spec.frames),
            &mut no_progress(),
        )
        .unwrap();
        let done = CHUNK_FRAMES; // one chunk in
        let partial = Peaks::partial(full.levels[0][..(done / BASE) as usize * 2].to_vec(), done);
        assert!(Peaks::partial(Vec::new(), 0).columns(0, 100, 10).is_empty());
        let cols = partial.columns(0, done, 64);
        assert_eq!(cols.len(), 128);
        // The same stretch from the finished build, at the same width, agrees.
        assert_eq!(cols, full.columns(0, done, 64));
    }

    #[test]
    fn coarser_levels_merge_four_buckets() {
        let frames = (BASE * 4 * 3000) as u32; // enough buckets to force a second level
        let spec = Spec {
            rate: 8000,
            channels: 1,
            bits: 8,
            frames,
        };
        let pcm = fixtures::pcm(spec, 2);
        let p = build(
            &mut &pcm[..],
            &spec.format(),
            u64::from(frames),
            &mut no_progress(),
        )
        .unwrap();
        assert!(p.levels.len() >= 2);
        let l0 = &p.levels[0];
        let expect: Vec<i8> = l0
            .chunks(8)
            .flat_map(|g| {
                [
                    g.as_chunks::<2>().0.iter().map(|b| b[0]).min().unwrap(),
                    g.as_chunks::<2>().0.iter().map(|b| b[1]).max().unwrap(),
                ]
            })
            .collect();
        assert_eq!(p.levels[1], expect);
        assert!(p.levels.last().unwrap().len() / 2 <= MIN_TOP_BUCKETS * FACTOR as usize);
    }

    #[test]
    fn columns_cover_the_requested_range_and_follow_the_signal() {
        // First half silent, second half loud: columns must show that boundary.
        let mut pcm = vec![0u8; 2048 * 4];
        for frame in pcm.as_chunks_mut::<4>().0.iter_mut().skip(1024) {
            frame[..2].copy_from_slice(&20000i16.to_be_bytes());
            frame[2..].copy_from_slice(&20000i16.to_be_bytes());
        }
        let p = build(
            &mut &pcm[..],
            &Spec::cd(0).format(),
            2048,
            &mut no_progress(),
        )
        .unwrap();
        let cols = p.columns(0, 2048, 8);
        assert_eq!(cols.len(), 16);
        assert!(
            cols[..8].iter().all(|&v| v == 0),
            "first half silent: {cols:?}"
        );
        assert!(
            cols[8..].chunks(2).all(|c| c[1] > 60),
            "second half loud: {cols:?}"
        );
        // Zooming in on only the loud half gives all-loud columns.
        assert!(p.columns(1024, 2048, 4).chunks(2).all(|c| c[1] > 60));
    }

    #[test]
    fn degenerate_ranges_are_empty_and_end_is_clamped() {
        let pcm = fixtures::pcm(Spec::cd(500), 1);
        let p = build(
            &mut &pcm[..],
            &Spec::cd(0).format(),
            500,
            &mut no_progress(),
        )
        .unwrap();
        assert!(p.columns(0, 500, 0).is_empty());
        assert!(p.columns(300, 300, 10).is_empty());
        assert!(p.columns(600, 700, 10).is_empty());
        assert_eq!(p.columns(0, 10_000, 5).len(), 10);
    }

    #[test]
    fn more_columns_than_frames_still_returns_every_column() {
        let pcm = fixtures::pcm(Spec::cd(100), 1);
        let p = build(
            &mut &pcm[..],
            &Spec::cd(0).format(),
            100,
            &mut no_progress(),
        )
        .unwrap();
        assert_eq!(p.columns(0, 100, 400).len(), 800);
    }

    #[test]
    fn progress_is_monotonic_and_cancel_stops_early() {
        let frames = 300_000u32;
        let pcm = vec![0u8; frames as usize * 4];
        let mut seen = Vec::new();
        build(
            &mut &pcm[..],
            &Spec::cd(0).format(),
            u64::from(frames),
            &mut |d, t| {
                seen.push((d, t));
                true
            },
        )
        .unwrap();
        assert!(seen.len() > 1 && seen.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(
            *seen.last().unwrap(),
            (u64::from(frames), u64::from(frames))
        );
        let r = build(
            &mut &pcm[..],
            &Spec::cd(0).format(),
            u64::from(frames),
            &mut |_, _| false,
        );
        assert!(matches!(r, Err(Error::Cancelled)));
    }

    #[test]
    fn a_short_source_is_an_error_not_a_panic() {
        let pcm = [0u8; 100];
        assert!(build(
            &mut &pcm[..],
            &Spec::cd(0).format(),
            1000,
            &mut no_progress()
        )
        .is_err());
    }

    #[test]
    fn memory_stays_flat_for_a_very_long_stream() {
        // 100 M frames (400 MB of PCM) generated on the fly: nothing proportional is allocated
        // beyond the peaks themselves (~1.6 MB).
        let frames = 100_000_000u64;
        let mut zeros = std::io::repeat(0).take(frames * 4);
        let p = build(
            &mut zeros,
            &Spec::cd(0).format(),
            frames,
            &mut no_progress(),
        )
        .unwrap();
        let total: usize = p.levels.iter().map(Vec::len).sum();
        assert!(total < 3_000_000, "{total} bytes of peaks");
    }

    #[test]
    fn builds_from_a_broken_file_without_repair() {
        let dir = tempfile::tempdir().unwrap();
        let spec = Spec::cd(2000);
        let path = dir.path().join("broken.aif");
        std::fs::write(&path, fixtures::broken(spec, 4, 3)).unwrap();
        let p = build_for_file(&path, &mut no_progress()).unwrap();
        assert_eq!(p.frames, 2000);
        assert_eq!(p.levels[0], naive(&fixtures::pcm(spec, 4), 4, 2, 2));
    }

    #[test]
    fn a_healthy_files_id3_trailer_is_not_drawn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ok.aif");
        std::fs::write(&path, fixtures::healthy(Spec::cd(1000), 1)).unwrap();
        assert_eq!(
            build_for_file(&path, &mut no_progress()).unwrap().frames,
            1000
        );
    }
}
