//! Loudness measurement after ITU-R BS.1770 / EBU R128: K-weighted energy in 100 ms steps, from
//! which momentary (400 ms), short-term (3 s) and gated integrated loudness follow.
//!
//! The analysis is one streaming pass that keeps only a few numbers per 100 ms, so a set of any
//! length costs a few megabytes.

use crate::biquad::Biquad;
use crate::truepeak::TruePeak;

/// LUFS offset of the BS.1770 formula.
const OFFSET: f64 = -0.691;
const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// Crossover between "bass" and "everything else", for the optional band statistics.
pub const BASS_HZ: f64 = 150.0;

pub fn lufs(energy: f64) -> f64 {
    if energy > 0.0 {
        OFFSET + 10.0 * energy.log10()
    } else {
        f64::NEG_INFINITY
    }
}

pub fn energy(lufs: f64) -> f64 {
    10f64.powf((lufs - OFFSET) / 10.0)
}

/// Per-100 ms measurements of a whole programme.
#[derive(Debug, Clone)]
pub struct Profile {
    pub rate: f64,
    pub channels: usize,
    /// Frames per step (rate / 10).
    pub hop_frames: usize,
    pub frames: u64,
    /// K-weighted mean square, summed over channels, per step.
    pub k_ms: Vec<f64>,
    /// Mean square of the part below `BASS_HZ` (unweighted, summed over channels), if measured.
    pub low_ms: Vec<f64>,
    /// Unweighted mean square of the whole signal (same scale as `low_ms`), if measured.
    pub full_ms: Vec<f64>,
    /// Highest true peak (linear) in each step.
    pub peak: Vec<f32>,
    /// Running sums of `k_ms`, for fast window averages.
    prefix: Vec<f64>,
}

const MAGIC: &[u8; 8] = b"RFLPROF1";

fn running_sums(k_ms: &[f64]) -> Vec<f64> {
    let mut prefix = Vec::with_capacity(k_ms.len() + 1);
    let mut sum = 0.0;
    prefix.push(0.0);
    for &e in k_ms {
        sum += e;
        prefix.push(sum);
    }
    prefix
}

impl Profile {
    /// Assemble a profile from measured steps (band statistics are not kept).
    pub fn from_parts(
        rate: f64,
        channels: usize,
        hop_frames: usize,
        frames: u64,
        k_ms: Vec<f64>,
        peak: Vec<f32>,
    ) -> Profile {
        let prefix = running_sums(&k_ms);
        Profile {
            rate,
            channels,
            hop_frames,
            frames,
            k_ms,
            low_ms: Vec::new(),
            full_ms: Vec::new(),
            peak,
            prefix,
        }
    }

    /// A compact form for the on-disk cache (energies as f32: far finer than any decision made from them).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(48 + self.hops() * 8);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.rate.to_le_bytes());
        out.extend_from_slice(&(self.channels as u32).to_le_bytes());
        out.extend_from_slice(&(self.hop_frames as u32).to_le_bytes());
        out.extend_from_slice(&self.frames.to_le_bytes());
        out.extend_from_slice(&(self.hops() as u64).to_le_bytes());
        for &e in &self.k_ms {
            out.extend_from_slice(&(e as f32).to_le_bytes());
        }
        for &p in &self.peak {
            out.extend_from_slice(&p.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Profile> {
        if b.len() < 40 || &b[..8] != MAGIC {
            return None;
        }
        let rate = f64::from_le_bytes(b[8..16].try_into().ok()?);
        let channels = u32::from_le_bytes(b[16..20].try_into().ok()?) as usize;
        let hop_frames = u32::from_le_bytes(b[20..24].try_into().ok()?) as usize;
        let frames = u64::from_le_bytes(b[24..32].try_into().ok()?);
        let n = u64::from_le_bytes(b[32..40].try_into().ok()?) as usize;
        if b.len() != 40 + n * 8 || hop_frames == 0 || !rate.is_finite() || rate <= 0.0 {
            return None;
        }
        let f32s = |from: usize| -> Vec<f32> {
            b[from..from + n * 4]
                .chunks(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        };
        let k_ms: Vec<f64> = f32s(40).into_iter().map(f64::from).collect();
        Some(Profile::from_parts(
            rate,
            channels,
            hop_frames,
            frames,
            k_ms,
            f32s(40 + n * 4),
        ))
    }

    /// The profile of several stretches played one after another: `(profile, start_frame, end_frame)`
    /// of the source each. Stretches rarely start on a step boundary, so each step of the result is
    /// the frame-weighted average of the source steps it overlaps. Accurate to a fraction of a step,
    /// which is far finer than the seconds-long windows the leveller works with. `None` if the
    /// stretches do not share one format or there are none.
    pub fn stitch(parts: &[(&Profile, u64, u64)]) -> Option<Profile> {
        let first = parts.first()?.0;
        if parts.iter().any(|(p, s, e)| {
            p.rate != first.rate
                || p.channels != first.channels
                || p.hop_frames != first.hop_frames
                || s >= e
        }) {
            return None;
        }
        let h = first.hop_frames as u64;
        let total: u64 = parts.iter().map(|(_, s, e)| e - s).sum();
        let hops = total.div_ceil(h) as usize;
        let (mut energy, mut weight, mut peak) =
            (vec![0.0f64; hops], vec![0u64; hops], vec![0f32; hops]);
        let mut at = 0u64; // where this stretch begins in the combined audio
        for &(p, start, end) in parts {
            for i in (start / h)..end.div_ceil(h).min(p.hops() as u64) {
                // The part of source step `i` inside the stretch, and where that lands in the result.
                let (a, b) = ((i * h).max(start), ((i + 1) * h).min(end));
                let (mut x, to) = (at + (a - start), at + (b - start));
                while x < to {
                    let j = x / h;
                    let upto = to.min((j + 1) * h);
                    energy[j as usize] += p.k_ms[i as usize] * (upto - x) as f64;
                    weight[j as usize] += upto - x;
                    peak[j as usize] = peak[j as usize].max(p.peak[i as usize]);
                    x = upto;
                }
            }
            at += end - start;
        }
        let k_ms = energy
            .iter()
            .zip(&weight)
            .map(|(&e, &w)| if w > 0 { e / w as f64 } else { 0.0 })
            .collect();
        Some(Profile::from_parts(
            first.rate,
            first.channels,
            first.hop_frames,
            total,
            k_ms,
            peak,
        ))
    }

    pub fn hops(&self) -> usize {
        self.k_ms.len()
    }

    pub fn hop_secs(&self) -> f64 {
        self.hop_frames as f64 / self.rate
    }

    pub fn duration_secs(&self) -> f64 {
        self.frames as f64 / self.rate
    }

    /// Mean energy over steps `[from, to)`, clamped to what exists.
    pub fn mean_energy(&self, from: isize, to: isize) -> f64 {
        let n = self.hops() as isize;
        let (a, b) = (from.clamp(0, n), to.clamp(0, n));
        if b <= a {
            return 0.0;
        }
        (self.prefix[b as usize] - self.prefix[a as usize]) / (b - a) as f64
    }

    /// Loudness of the 3 s window centred on step `i` (shorter at the ends).
    pub fn short_term(&self, i: usize) -> f64 {
        let i = i as isize;
        lufs(self.mean_energy(i - 14, i + 16))
    }

    /// Gated integrated loudness, or -inf if the programme is silent.
    pub fn integrated(&self) -> f64 {
        let blocks = self.blocks();
        let abs_gate = energy(ABSOLUTE_GATE_LUFS);
        let above: Vec<f64> = blocks.iter().copied().filter(|&e| e > abs_gate).collect();
        if above.is_empty() {
            return f64::NEG_INFINITY;
        }
        let mean = above.iter().sum::<f64>() / above.len() as f64;
        let rel_gate = mean * 0.1; // 10 LU below
        let kept: Vec<f64> = above.into_iter().filter(|&e| e > rel_gate).collect();
        lufs(kept.iter().sum::<f64>() / kept.len().max(1) as f64)
    }

    /// 400 ms block energies with 75 % overlap (one per step).
    fn blocks(&self) -> Vec<f64> {
        let n = self.hops();
        // The last, possibly short, step is left out: a block must be a full 400 ms.
        let full = if (self.frames as usize).is_multiple_of(self.hop_frames) {
            n
        } else {
            n.saturating_sub(1)
        };
        (0..full.saturating_sub(3))
            .map(|i| self.mean_energy(i as isize, i as isize + 4))
            .collect()
    }

    /// Loudness range (EBU Tech 3342): spread of the short-term loudness, ignoring quiet parts.
    pub fn loudness_range(&self) -> f64 {
        let st: Vec<f64> = (0..self.hops()).map(|i| self.short_term(i)).collect();
        let gate = energy(ABSOLUTE_GATE_LUFS);
        let above: Vec<f64> = st.into_iter().filter(|&l| energy(l) > gate).collect();
        if above.len() < 2 {
            return 0.0;
        }
        let mean = above.iter().map(|&l| energy(l)).sum::<f64>() / above.len() as f64;
        let rel = lufs(mean) - 20.0;
        let mut kept: Vec<f64> = above.into_iter().filter(|&l| l > rel).collect();
        if kept.len() < 2 {
            return 0.0;
        }
        kept.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let at = |p: f64| kept[((kept.len() - 1) as f64 * p).round() as usize];
        at(0.95) - at(0.10)
    }

    /// Highest true peak in the programme, linear.
    pub fn peak_max(&self) -> f32 {
        self.peak.iter().copied().fold(0.0, f32::max)
    }
}

/// Streaming analyser: feed it interleaved f32 frames in any chunking.
pub struct Analyzer {
    rate: f64,
    channels: usize,
    hop_frames: usize,
    shelf: Vec<Biquad>,
    highpass: Vec<Biquad>,
    lowpass: Option<Vec<[Biquad; 2]>>,
    truepeak: TruePeak,
    in_hop: usize,
    acc_k: f64,
    acc_low: f64,
    acc_full: f64,
    hop_peak: f32,
    /// Highest true peak so far, to decide when the interpolation filter is worth running.
    peak_so_far: f32,
    frames: u64,
    k_ms: Vec<f64>,
    low_ms: Vec<f64>,
    full_ms: Vec<f64>,
    peak: Vec<f32>,
    scratch: Vec<f32>,
}

impl Analyzer {
    /// `bands`: also measure the energy below `BASS_HZ` (a little extra work per frame).
    pub fn new(rate: f64, channels: usize, bands: bool) -> Analyzer {
        let hop_frames = ((rate / 10.0).round() as usize).max(1);
        Analyzer {
            rate,
            channels,
            hop_frames,
            shelf: vec![Biquad::k_shelf(rate); channels],
            highpass: vec![Biquad::k_highpass(rate); channels],
            lowpass: bands.then(|| vec![[Biquad::lowpass(rate, BASS_HZ); 2]; channels]),
            truepeak: TruePeak::new(channels),
            in_hop: 0,
            acc_k: 0.0,
            acc_low: 0.0,
            acc_full: 0.0,
            hop_peak: 0.0,
            peak_so_far: 0.0,
            frames: 0,
            k_ms: Vec::new(),
            low_ms: Vec::new(),
            full_ms: Vec::new(),
            peak: Vec::new(),
            scratch: Vec::new(),
        }
    }

    fn close_hop(&mut self) {
        let n = self.in_hop.max(1) as f64;
        self.k_ms.push(self.acc_k / n);
        if self.lowpass.is_some() {
            self.low_ms.push(self.acc_low / n);
            self.full_ms.push(self.acc_full / n);
        }
        self.peak.push(self.hop_peak);
        self.in_hop = 0;
        self.acc_k = 0.0;
        self.acc_low = 0.0;
        self.acc_full = 0.0;
        self.hop_peak = 0.0;
    }

    pub fn push(&mut self, interleaved: &[f32]) {
        // Inter-sample peaks only matter once a window could beat the loudest peak so far, so the
        // interpolation filter is skipped for everything quieter than that.
        let ch = self.channels;
        for frame in interleaved.chunks_exact(ch) {
            for (c, &x) in frame.iter().enumerate() {
                let w = self.highpass[c].process(self.shelf[c].process(f64::from(x)));
                self.acc_k += w * w;
                if let Some(lp) = &mut self.lowpass {
                    let first = lp[c][0].process(f64::from(x));
                    let l = lp[c][1].process(first);
                    self.acc_low += l * l;
                    self.acc_full += f64::from(x) * f64::from(x);
                }
            }
            if let Some(p) = self.truepeak.push(frame, self.peak_so_far) {
                self.hop_peak = self.hop_peak.max(p);
                self.peak_so_far = self.peak_so_far.max(p);
            }
            self.in_hop += 1;
            self.frames += 1;
            if self.in_hop == self.hop_frames {
                self.close_hop();
            }
        }
    }

    /// Feed integer-valued samples scaled to +-1 (convenience for tests and tools).
    pub fn push_scaled(&mut self, samples: impl IntoIterator<Item = f32>) {
        self.scratch.clear();
        self.scratch.extend(samples);
        let buf = std::mem::take(&mut self.scratch);
        self.push(&buf);
        self.scratch = buf;
    }

    pub fn finish(mut self) -> Profile {
        // The true-peak detector lags a few frames behind; flush it so the last peaks count.
        let zeros = vec![0.0f32; self.channels];
        for _ in 0..crate::truepeak::DELAY {
            if let Some(p) = self.truepeak.push(&zeros, self.peak_so_far) {
                self.hop_peak = self.hop_peak.max(p);
            }
        }
        if self.in_hop > 0 {
            self.close_hop();
        }
        let prefix = running_sums(&self.k_ms);
        Profile {
            rate: self.rate,
            channels: self.channels,
            hop_frames: self.hop_frames,
            frames: self.frames,
            k_ms: self.k_ms,
            low_ms: self.low_ms,
            full_ms: self.full_ms,
            peak: self.peak,
            prefix,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Stereo (same in both channels) 997 Hz sine at `db` dBFS peak, for `secs`.
    fn sine(rate: f64, db: f64, secs: f64) -> Vec<f32> {
        let a = 10f64.powf(db / 20.0);
        (0..(rate * secs) as usize)
            .flat_map(|i| {
                let v = (a * (2.0 * PI * 997.0 * i as f64 / rate).sin()) as f32;
                [v, v]
            })
            .collect()
    }

    fn analyse(rate: f64, samples: &[f32]) -> Profile {
        let mut a = Analyzer::new(rate, 2, false);
        // Awkward chunking must not matter.
        for chunk in samples.chunks(2 * 1237) {
            a.push(chunk);
        }
        a.finish()
    }

    #[test]
    fn a_997_hz_sine_at_minus_23_dbfs_reads_minus_23_lufs() {
        for rate in [44_100.0, 48_000.0, 22_050.0] {
            let p = analyse(rate, &sine(rate, -23.0, 20.0));
            assert!(
                (p.integrated() + 23.0).abs() < 0.1,
                "{rate}: {}",
                p.integrated()
            );
            assert!((p.short_term(50) + 23.0).abs() < 0.1);
            assert!(p.loudness_range() < 0.2);
            let peak_db = 20.0 * f64::from(p.peak_max()).log10();
            assert!((peak_db + 23.0).abs() < 0.2, "{peak_db}");
        }
    }

    #[test]
    fn level_changes_move_the_reading_one_for_one() {
        let rate = 44_100.0;
        let a = analyse(rate, &sine(rate, -30.0, 10.0)).integrated();
        let b = analyse(rate, &sine(rate, -20.0, 10.0)).integrated();
        assert!((b - a - 10.0).abs() < 0.05);
    }

    #[test]
    fn silence_does_not_pull_the_integrated_value_down() {
        let rate = 44_100.0;
        let mut s = sine(rate, -20.0, 10.0);
        let plain = analyse(rate, &s).integrated();
        s.extend(vec![0.0; (rate * 2.0 * 30.0) as usize]); // 30 s of nothing
                                                           // Blocks that straddle the edge keep a little energy, as the standard defines.
        assert!((analyse(rate, &s).integrated() - plain).abs() < 0.15);
    }

    #[test]
    fn quiet_passages_are_gated_out_of_the_integrated_value() {
        // 10 s at -20 then 10 s at -40: the -40 part is more than 10 LU below and ignored.
        let rate = 44_100.0;
        let mut s = sine(rate, -20.0, 10.0);
        s.extend(sine(rate, -40.0, 10.0));
        assert!((analyse(rate, &s).integrated() + 20.0).abs() < 0.1);
    }

    #[test]
    fn loudness_range_sees_a_step() {
        let rate = 44_100.0;
        let mut s = sine(rate, -26.0, 20.0);
        s.extend(sine(rate, -16.0, 20.0));
        let lra = analyse(rate, &s).loudness_range();
        assert!((lra - 10.0).abs() < 1.0, "{lra}");
    }

    #[test]
    fn the_last_partial_step_is_kept_and_counted() {
        let rate = 44_100.0;
        let p = analyse(rate, &sine(rate, -20.0, 1.55));
        assert_eq!(p.hops(), 16);
        assert_eq!(p.frames, (rate * 1.55) as u64);
    }

    #[test]
    fn a_profile_survives_the_cache_file() {
        let rate = 44_100.0;
        let p = analyse(rate, &sine(rate, -20.0, 5.0));
        let q = Profile::from_bytes(&p.to_bytes()).unwrap();
        assert_eq!(
            (q.hops(), q.frames, q.hop_frames),
            (p.hops(), p.frames, p.hop_frames)
        );
        assert!((q.integrated() - p.integrated()).abs() < 1e-4);
        assert_eq!(q.peak, p.peak);
        assert!(Profile::from_bytes(b"junk").is_none());
        let mut cut = p.to_bytes();
        cut.pop();
        assert!(Profile::from_bytes(&cut).is_none());
    }

    #[test]
    fn stitching_follows_the_audio_not_the_step_grid() {
        // 3.37 s of a -20 source then 3.1 s of a -30 one, neither starting on a step boundary.
        let rate = 44_100.0;
        let pa = analyse(rate, &sine(rate, -20.0, 6.0));
        let pb = analyse(rate, &sine(rate, -30.0, 6.0));
        let (la, lb) = ((3.37 * rate) as u64, (3.1 * rate) as u64);
        let s = Profile::stitch(&[(&pa, 22_000, 22_000 + la), (&pb, 9_000, 9_000 + lb)]).unwrap();
        assert_eq!(s.frames, la + lb);
        assert_eq!(s.hops(), (s.frames as usize).div_ceil(s.hop_frames));
        let level = |t: f64| lufs(s.k_ms[(t / s.hop_secs()) as usize]);
        assert!((level(1.0) + 20.0).abs() < 0.1, "{}", level(1.0));
        assert!((level(5.0) + 30.0).abs() < 0.1, "{}", level(5.0));
        // The join is at 3.37 s: still the first level just before it, the second just after.
        assert!(
            level(3.25) > -20.1 && level(3.5) < -29.9,
            "{} {}",
            level(3.25),
            level(3.5)
        );
        assert!(s.peak.iter().all(|&p| p > 0.0));
    }

    #[test]
    fn stitching_refuses_mixed_formats_and_empty_input() {
        let p = analyse(44_100.0, &sine(44_100.0, -20.0, 2.0));
        let q = analyse(48_000.0, &sine(48_000.0, -20.0, 2.0));
        assert!(Profile::stitch(&[(&p, 0, 1000), (&q, 0, 1000)]).is_none());
        assert!(Profile::stitch(&[]).is_none());
        assert!(Profile::stitch(&[(&p, 500, 500)]).is_none());
    }

    #[test]
    fn bass_energy_is_reported_separately() {
        let rate = 44_100.0;
        let tone = |f: f64| -> Vec<f32> {
            (0..(rate * 5.0) as usize)
                .flat_map(|i| {
                    let v = (0.3 * (2.0 * PI * f * i as f64 / rate).sin()) as f32;
                    [v, v]
                })
                .collect()
        };
        let mut a = Analyzer::new(rate, 2, true);
        a.push(&tone(60.0));
        let low = a.finish();
        let mut a = Analyzer::new(rate, 2, true);
        a.push(&tone(3000.0));
        let high = a.finish();
        assert!(low.low_ms[10] > 50.0 * high.low_ms[10]);
    }
}
