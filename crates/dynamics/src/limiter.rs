//! A true-peak brickwall limiter with lookahead, built to be unobtrusive rather than loud.
//!
//! How it avoids sounding like a limiter:
//! * It sees each peak `lookahead` ahead, takes the lowest gain wanted over that span, and smooths it
//!   with two moving averages, so gain reduction glides in over a few milliseconds instead of
//!   slamming (the smoothing is no wider than the span, which is what guarantees the ceiling holds).
//! * Gain eases back up at a program-dependent rate: fast after an isolated peak, slow while the
//!   music is being limited continuously, so sustained limiting does not pump.
//! * One gain is applied to every channel, so the stereo image does not move.
//! * Below the ceiling the signal passes through untouched (gain exactly 1.0).
//!
//! The ceiling is a *true-peak* ceiling: the detector looks between samples (see `truepeak`).

use crate::truepeak::{self, TruePeak};
use std::collections::VecDeque;

/// Safety margin under the requested ceiling, in dB, for the small error of the 4x detector.
const MARGIN_DB: f64 = 0.05;
/// How often the slowly-changing release rate and statistics are updated, in frames.
const CONTROL_EVERY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimiterConfig {
    pub ceiling_db: f64,
    pub lookahead_ms: f64,
    pub release_fast_ms: f64,
    pub release_slow_ms: f64,
}

impl LimiterConfig {
    pub fn new(ceiling_db: f64) -> LimiterConfig {
        LimiterConfig {
            ceiling_db,
            lookahead_ms: 5.0,
            release_fast_ms: 40.0,
            release_slow_ms: 600.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LimiterStats {
    /// Deepest gain reduction, dB (positive).
    pub max_reduction_db: f64,
    /// Mean gain reduction over the whole programme, dB.
    pub mean_reduction_db: f64,
    /// Fraction of time (0..1) the reduction was above 0.1 dB.
    pub active_fraction: f64,
}

struct Boxcar {
    buf: Vec<f32>,
    pos: usize,
    sum: f64,
}

impl Boxcar {
    fn new(len: usize) -> Boxcar {
        Boxcar {
            buf: vec![1.0; len],
            pos: 0,
            sum: len as f64,
        }
    }

    #[inline]
    fn push(&mut self, x: f32) -> f32 {
        self.sum += f64::from(x) - f64::from(self.buf[self.pos]);
        self.buf[self.pos] = x;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        (self.sum / self.buf.len() as f64) as f32
    }
}

pub struct Limiter {
    channels: usize,
    rate: f64,
    cfg: LimiterConfig,
    ceiling: f32,
    tp: TruePeak,
    /// Window over which the lowest wanted gain is taken.
    span: usize,
    /// Frames the output lags the input.
    delay: usize,
    ring: Vec<f32>,
    ring_pos: usize,
    /// The frame that just left the delay line.
    old: Vec<f32>,
    min_q: VecDeque<(u64, f32)>,
    index: u64,
    box1: Boxcar,
    box2: Boxcar,
    current: f32,
    release_coef: f32,
    avg_reduction_db: f64,
    frames_in: u64,
    // statistics
    lowest: f32,
    sampled: u64,
    sum_db: f64,
    active: u64,
    /// Output frames still to be dropped (the silence that primes the delay line).
    skip: usize,
}

impl Limiter {
    pub fn new(rate: f64, channels: usize, cfg: LimiterConfig) -> Limiter {
        let span = ((cfg.lookahead_ms / 1000.0 * rate).round() as usize).max(4);
        let b1 = span.div_ceil(2);
        let b2 = span - b1 + 1;
        let delay = truepeak::DELAY + span - 1;
        Limiter {
            channels,
            rate,
            cfg,
            ceiling: 10f64.powf((cfg.ceiling_db - MARGIN_DB) / 20.0) as f32,
            tp: TruePeak::new(channels),
            span,
            delay,
            ring: vec![0.0; delay * channels],
            ring_pos: 0,
            old: vec![0.0; channels],
            min_q: VecDeque::new(),
            index: 0,
            box1: Boxcar::new(b1),
            box2: Boxcar::new(b2),
            current: 1.0,
            release_coef: Self::coef(rate, cfg.release_fast_ms),
            avg_reduction_db: 0.0,
            frames_in: 0,
            lowest: 1.0,
            sampled: 0,
            sum_db: 0.0,
            active: 0,
            skip: delay,
        }
    }

    fn coef(rate: f64, ms: f64) -> f32 {
        (1.0 - (-1.0 / (ms / 1000.0 * rate)).exp()) as f32
    }

    /// Process whole interleaved frames, appending the limited audio to `out`. The first call(s)
    /// yield fewer frames than went in; `finish` returns the rest.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let ch = self.channels;
        out.reserve(input.len());
        for frame in input.chunks_exact(ch) {
            self.step(frame, out);
        }
    }

    /// Flush the frames still inside the lookahead.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        let zeros = vec![0.0f32; self.channels];
        for _ in 0..self.delay {
            self.step(&zeros, out);
        }
    }

    #[inline]
    fn step(&mut self, frame: &[f32], out: &mut Vec<f32>) {
        let ch = self.channels;
        // The frame that left the delay line, and the new one taking its place.
        let slot = self.ring_pos * ch;
        self.old.copy_from_slice(&self.ring[slot..slot + ch]);
        self.ring[slot..slot + ch].copy_from_slice(frame);
        self.ring_pos += 1;
        if self.ring_pos == self.delay {
            self.ring_pos = 0;
        }

        // Gain this frame's peak calls for.
        let wanted = match self.tp.push(frame, self.ceiling) {
            Some(d) if d > self.ceiling => self.ceiling / d,
            _ => 1.0,
        };
        // Lowest wanted gain over the last `span` detections.
        while self.min_q.back().is_some_and(|&(_, v)| v >= wanted) {
            self.min_q.pop_back();
        }
        self.min_q.push_back((self.index, wanted));
        while self
            .min_q
            .front()
            .is_some_and(|&(i, _)| i + self.span as u64 <= self.index)
        {
            self.min_q.pop_front();
        }
        self.index += 1;
        let lowest = self.min_q.front().map_or(1.0, |&(_, v)| v);
        let smooth = self.box2.push(self.box1.push(lowest));

        // Ease back up slowly, but never above what the smoothed curve allows.
        let rising = self.current + (1.0 - self.current) * self.release_coef;
        self.current = smooth.min(rising);
        self.lowest = self.lowest.min(self.current);

        self.frames_in += 1;
        if self.frames_in.is_multiple_of(CONTROL_EVERY as u64) {
            self.control();
        }

        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        let g = self.current;
        out.extend(self.old.iter().map(|&x| x * g));
    }

    fn control(&mut self) {
        let reduction = -20.0 * f64::from(self.current).log10();
        self.sampled += 1;
        self.sum_db += reduction;
        if reduction > 0.1 {
            self.active += 1;
        }
        // Average reduction over about a second decides how slowly to let go.
        let alpha = CONTROL_EVERY as f64 / self.rate;
        self.avg_reduction_db += (reduction - self.avg_reduction_db) * alpha;
        let busy = (self.avg_reduction_db / 4.0).clamp(0.0, 1.0);
        let ms =
            self.cfg.release_fast_ms + (self.cfg.release_slow_ms - self.cfg.release_fast_ms) * busy;
        self.release_coef = Self::coef(self.rate, ms);
    }

    pub fn stats(&self) -> LimiterStats {
        let n = self.sampled.max(1) as f64;
        LimiterStats {
            max_reduction_db: -20.0 * f64::from(self.lowest).log10(),
            mean_reduction_db: self.sum_db / n,
            active_fraction: self.active as f64 / n,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biquad::Biquad;
    use std::f64::consts::PI;

    const RATE: f64 = 44_100.0;

    fn run(
        input: &[f32],
        channels: usize,
        ceiling_db: f64,
        chunk: usize,
    ) -> (Vec<f32>, LimiterStats) {
        let mut l = Limiter::new(RATE, channels, LimiterConfig::new(ceiling_db));
        let mut out = Vec::new();
        for c in input.chunks(chunk * channels) {
            l.process(c, &mut out);
        }
        l.finish(&mut out);
        (out, l.stats())
    }

    /// Highest level between samples, measured with a much longer interpolator at 16x.
    fn reference_true_peak(x: &[f32]) -> f64 {
        let n = x.len() as isize;
        let mut best = 0f64;
        for i in 0..n {
            best = best.max(f64::from(x[i as usize].abs()));
            for sub in 1..16 {
                let t = f64::from(sub) / 16.0;
                let mut acc = 0.0;
                for k in -32..=33isize {
                    let j = i + k;
                    if j < 0 || j >= n {
                        continue;
                    }
                    let d = k as f64 - t;
                    let sinc = (PI * d).sin() / (PI * d);
                    let lanczos = (PI * d / 33.0).sin() / (PI * d / 33.0);
                    acc += f64::from(x[j as usize]) * sinc * lanczos;
                }
                best = best.max(acc.abs());
            }
        }
        best
    }

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f64) / (1u64 << 31) as f64 * 2.0 - 1.0
        }
    }

    fn adversarial() -> Vec<(&'static str, Vec<f32>)> {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let mut rng = Lcg(7);
        vec![
            (
                "inter-sample sine, +9 dB",
                (0..30_000)
                    .map(|i| (3.0 * s * [1.0, 1.0, -1.0, -1.0][i % 4]) as f32)
                    .collect(),
            ),
            ("loud band-limited noise bursts", {
                // Music has little above 16 kHz; white noise right up to Nyquist overshoots between
                // samples in ways no 4x detector (BS.1770's included) is meant to catch.
                let mut lp = [Biquad::lowpass(RATE, 9000.0), Biquad::lowpass(RATE, 9000.0)];
                (0..60_000)
                    .map(|i| {
                        let x = if (i / 7000) % 2 == 0 {
                            rng.next() * 4.0
                        } else {
                            rng.next() * 0.1
                        };
                        {
                            let a = lp[0].process(x);
                            lp[1].process(a) as f32
                        }
                    })
                    .collect()
            }),
            (
                "full-scale square x2",
                (0..30_000)
                    .map(|i| if (i / 50) % 2 == 0 { 2.0 } else { -2.0 })
                    .collect(),
            ),
            (
                "60 Hz bass at +6 dB",
                (0..80_000)
                    .map(|i| (2.0 * (2.0 * PI * 60.0 * i as f64 / RATE).sin()) as f32)
                    .collect(),
            ),
            (
                "single spike",
                (0..20_000)
                    .map(|i| if i == 10_000 { 8.0 } else { 0.0 })
                    .collect(),
            ),
        ]
    }

    #[test]
    fn the_true_peak_ceiling_holds_on_nasty_signals() {
        for ceiling in [-0.1, -1.0, -3.0] {
            for (name, sig) in adversarial() {
                let (out, _) = run(&sig, 1, ceiling, 4096);
                assert_eq!(out.len(), sig.len(), "{name}");
                let peak = db(reference_true_peak(&out));
                // A hard-edged square wave rings (Gibbs) further than the 12-tap detector reaches; allow it a
                // little more. Everything music-like must be within 0.05 dB.
                let slack = if name.starts_with("full-scale square") {
                    0.3
                } else {
                    0.05
                };
                assert!(
                    peak <= ceiling + slack,
                    "{name} @ {ceiling}: true peak {peak:.3} dB"
                );
            }
        }
    }

    #[test]
    fn stereo_is_limited_together_and_keeps_its_balance() {
        // Left loud, right quiet: both get the same gain.
        let sig: Vec<f32> = (0..40_000)
            .flat_map(|i| {
                let v = (2.0 * (2.0 * PI * 200.0 * i as f64 / RATE).sin()) as f32;
                [v, v * 0.25]
            })
            .collect();
        let (out, _) = run(&sig, 2, -1.0, 1000);
        for (o, s) in out.chunks(2).zip(sig.chunks(2)).skip(4000).step_by(97) {
            if s[0].abs() > 0.2 {
                let (gl, gr) = (o[0] / s[0], o[1] / s[1]);
                assert!((gl - gr).abs() < 1e-5, "{gl} {gr}");
            }
        }
    }

    #[test]
    fn audio_under_the_ceiling_comes_back_unchanged_and_in_time() {
        let sig: Vec<f32> = (0..50_000)
            .map(|i| (0.5 * (2.0 * PI * 440.0 * i as f64 / RATE).sin()) as f32)
            .collect();
        let (out, stats) = run(&sig, 1, -1.0, 777);
        assert_eq!(out, sig);
        assert_eq!(stats.max_reduction_db, 0.0);
    }

    #[test]
    fn chunking_never_changes_the_result() {
        let (_, sig) = adversarial().remove(1);
        let a = run(&sig, 1, -1.0, 64).0;
        let b = run(&sig, 1, -1.0, 5000).0;
        assert_eq!(a, b);
    }

    #[test]
    fn a_lone_peak_is_pulled_down_gently_and_only_around_itself() {
        let mut sig = vec![0.0f32; 40_000];
        // A quiet tone with one peak 8 dB over the ceiling.
        for (i, v) in sig.iter_mut().enumerate() {
            *v = (0.2 * (2.0 * PI * 440.0 * i as f64 / RATE).sin()) as f32;
        }
        sig[20_000] = 2.0;
        let (out, stats) = run(&sig, 1, -1.0, 4096);
        assert!(out[20_000].abs() <= 0.9);
        assert!(stats.max_reduction_db > 6.0);
        // Far from the peak the tone is untouched once the gain has recovered.
        assert_eq!(&out[..15_000], &sig[..15_000]);
        assert!((out[39_000] - sig[39_000]).abs() < 1e-3);
        // The reduction glides: neighbouring samples never jump by more than a few percent of gain.
        let ratio = |i: usize| {
            if sig[i].abs() > 0.05 {
                out[i] / sig[i]
            } else {
                1.0
            }
        };
        for i in 19_700..20_300 {
            if sig[i].abs() > 0.05 && sig[i + 1].abs() > 0.05 && (i + 1) != 20_000 && i != 20_000 {
                assert!((ratio(i + 1) - ratio(i)).abs() < 0.08, "gain jumped at {i}");
            }
        }
    }

    #[test]
    fn steady_limiting_of_bass_adds_little_distortion() {
        // 60 Hz at +4 dB over the ceiling: the textbook case for a limiter that distorts bass.
        let sig: Vec<f32> = (0..RATE as usize * 3)
            .map(|i| (1.6 * (2.0 * PI * 60.0 * i as f64 / RATE).sin()) as f32)
            .collect();
        let (out, stats) = run(&sig, 1, -1.0, 4096);
        assert!(stats.max_reduction_db > 3.0);
        // Skip the first second, then compare the energy at 60 Hz with everything else.
        let tail = &out[RATE as usize * 2..];
        let mut fundamental = (0.0, 0.0);
        let mut total = 0.0;
        for (i, &v) in tail.iter().enumerate() {
            let ph = 2.0 * PI * 60.0 * i as f64 / RATE;
            fundamental.0 += f64::from(v) * ph.sin();
            fundamental.1 += f64::from(v) * ph.cos();
            total += f64::from(v) * f64::from(v);
        }
        let n = tail.len() as f64;
        let fund_power = 2.0 * (fundamental.0.powi(2) + fundamental.1.powi(2)) / (n * n);
        let thd = ((total / n - fund_power).max(0.0) / fund_power).sqrt();
        assert!(thd < 0.03, "distortion {:.2} %", thd * 100.0);
    }

    #[test]
    fn it_lets_go_after_a_loud_passage() {
        // 2 s of over-level tone, then a quiet tone: gain must be back to 1 within about a second.
        let sig: Vec<f32> = (0..RATE as usize * 5)
            .map(|i| {
                let a = if i < RATE as usize * 2 { 2.0 } else { 0.1 };
                (a * (2.0 * PI * 1000.0 * i as f64 / RATE).sin()) as f32
            })
            .collect();
        let (out, _) = run(&sig, 1, -1.0, 4096);
        let at = |sec: f64| {
            let i = (sec * RATE) as usize;
            let (o, s) = (
                out[i..i + 44].iter().fold(0f32, |m, v| m.max(v.abs())),
                sig[i..i + 44].iter().fold(0f32, |m, v| m.max(v.abs())),
            );
            o / s
        };
        assert!(at(1.5) < 0.6);
        assert!(at(4.8) > 0.999, "{}", at(4.8));
    }

    #[test]
    fn mono_stereo_and_many_channels_all_work() {
        for ch in [1usize, 2, 6] {
            let sig: Vec<f32> = (0..3000 * ch)
                .map(|i| if i % 97 == 0 { 3.0 } else { 0.1 })
                .collect();
            let (out, _) = run(&sig, ch, -1.0, 100);
            assert_eq!(out.len(), sig.len());
            assert!(out.iter().all(|v| v.abs() <= 1.0));
        }
    }
}
