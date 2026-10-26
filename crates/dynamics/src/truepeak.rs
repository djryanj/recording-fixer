//! True-peak detection: the highest level the waveform reaches *between* samples, found by 4x
//! oversampling with a windowed-sinc interpolator (BS.1770 Annex 2 describes the same idea).
//!
//! A DAC, a sample-rate converter or an MP3/AAC decoder reconstructs that curve, so a file whose
//! samples stay under 0 dBFS can still clip when played. That is why the limiter's ceiling is a
//! true-peak ceiling.

use std::f64::consts::PI;

const TAPS: usize = 12;
/// Samples before the interval being interpolated that the filter reaches.
const BEFORE: usize = 5;
/// Frames a detection lags behind the newest frame pushed.
pub const DELAY: usize = TAPS - BEFORE - 1;

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// Zeroth-order modified Bessel function, for the Kaiser window.
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..40 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
    }
    sum
}

pub struct TruePeak {
    channels: usize,
    /// Interpolator taps for the three in-between positions (1/4, 1/2, 3/4).
    phases: [[f32; TAPS]; 3],
    /// Largest sum of absolute tap values: no interpolated value can exceed this times the
    /// largest sample in the window, which lets quiet stretches skip the filter.
    bound: f32,
    /// Last `TAPS` frames per channel, oldest first.
    window: Vec<f32>,
    pushed: u64,
}

impl TruePeak {
    pub fn new(channels: usize) -> TruePeak {
        let mut phases = [[0f32; TAPS]; 3];
        let beta = 8.0;
        let half = (TAPS / 2) as f64;
        for (p, taps) in phases.iter_mut().enumerate() {
            let offset = (p + 1) as f64 / 4.0;
            let raw: Vec<f64> = (0..TAPS)
                .map(|j| {
                    let t = j as f64 - BEFORE as f64 - offset;
                    let w = bessel_i0(beta * (1.0 - (t / half).powi(2)).max(0.0).sqrt())
                        / bessel_i0(beta);
                    sinc(t) * w
                })
                .collect();
            let sum: f64 = raw.iter().sum();
            for (j, v) in raw.iter().enumerate() {
                taps[j] = (v / sum) as f32;
            }
        }
        let bound = phases
            .iter()
            .map(|t| t.iter().map(|v| v.abs()).sum::<f32>())
            .fold(1.0, f32::max);
        TruePeak {
            channels,
            phases,
            bound,
            window: vec![0.0; channels * TAPS],
            pushed: 0,
        }
    }

    /// Push one frame (one sample per channel). Once enough frames are in, returns the true peak of
    /// the stretch starting at the frame `DELAY` frames back. When the filter's reach says it cannot
    /// exceed `skip_below`, the plain sample peak is returned instead (a cheap lower bound).
    pub fn push(&mut self, frame: &[f32], skip_below: f32) -> Option<f32> {
        debug_assert_eq!(frame.len(), self.channels);
        for (c, &x) in frame.iter().enumerate() {
            let w = &mut self.window[c * TAPS..(c + 1) * TAPS];
            w.copy_within(1.., 0);
            w[TAPS - 1] = x;
        }
        self.pushed += 1;
        if self.pushed <= DELAY as u64 {
            return None;
        }
        let mut peak = 0f32;
        let mut window_max = 0f32;
        for c in 0..self.channels {
            let w = &self.window[c * TAPS..(c + 1) * TAPS];
            peak = peak.max(w[BEFORE].abs());
            window_max = w.iter().fold(window_max, |m, v| m.max(v.abs()));
        }
        if window_max * self.bound < skip_below {
            return Some(peak);
        }
        for c in 0..self.channels {
            let w = &self.window[c * TAPS..(c + 1) * TAPS];
            for taps in &self.phases {
                let y: f32 = taps.iter().zip(w).map(|(h, x)| h * x).sum();
                peak = peak.max(y.abs());
            }
        }
        Some(peak)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak_of(frames: &[[f32; 1]]) -> f32 {
        let mut tp = TruePeak::new(1);
        let mut best = 0f32;
        for f in frames.iter().chain(std::iter::repeat_n(&[0.0], DELAY + 1)) {
            if let Some(p) = tp.push(f, 0.0) {
                best = best.max(p);
            }
        }
        best
    }

    #[test]
    fn a_quarter_rate_sine_at_45_degrees_hides_a_3_db_peak() {
        // Samples of +-0.7071 for a sine whose true amplitude is 1.0: the classic inter-sample case.
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let frames: Vec<[f32; 1]> = (0..400).map(|i| [[s, s, -s, -s][i % 4]]).collect();
        let sample_peak = s;
        let tp = peak_of(&frames);
        assert!(tp > 0.98 && tp < 1.02, "true peak {tp}");
        assert!(tp > sample_peak * 1.35);
    }

    #[test]
    fn matches_a_much_finer_reference_on_noise_like_material() {
        // Compare with the exact continuous peak of a sum of sines (evaluated at 64 points per sample).
        let tones = [(0.31, 0.5), (0.43, 0.3), (0.19, 0.4)];
        let value = |t: f64| -> f64 {
            tones
                .iter()
                .map(|&(f, a)| a * (2.0 * PI * f * t).sin())
                .sum()
        };
        let frames: Vec<[f32; 1]> = (0..2000).map(|i| [value(i as f64) as f32]).collect();
        let truth = (0..2000 * 64)
            .map(|k| value(k as f64 / 64.0).abs())
            .fold(0.0, f64::max);
        let tp = f64::from(peak_of(&frames));
        let sample_peak = frames.iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        assert!((tp - truth).abs() < 0.01 * truth, "tp {tp} truth {truth}");
        assert!(tp >= f64::from(sample_peak) - 1e-6);
    }

    #[test]
    fn skipping_never_hides_a_peak_that_matters() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let mut tp = TruePeak::new(1);
        let mut best = 0f32;
        for i in 0..400 {
            if let Some(p) = tp.push(&[[s, s, -s, -s][i % 4]], 0.9) {
                best = best.max(p);
            }
        }
        assert!(best > 0.98);
    }
}
