//! Second-order filter sections, in f64 so the low frequencies stay accurate.

use std::f64::consts::PI;

#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn new(b: [f64; 3], a1: f64, a2: f64) -> Biquad {
        Biquad {
            b0: b[0],
            b1: b[1],
            b2: b[2],
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Stage 1 of BS.1770 K-weighting: the head-related high shelf (about +4 dB above 1.7 kHz).
    pub fn k_shelf(rate: f64) -> Biquad {
        let (f0, gain_db, q) = (
            1_681.974_450_955_533,
            3.999_843_853_973_347,
            0.707_175_236_955_419_6,
        );
        let k = (PI * f0 / rate).tan();
        let vh = 10f64.powf(gain_db / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        Biquad::new(
            [
                (vh + vb * k / q + k * k) / a0,
                2.0 * (k * k - vh) / a0,
                (vh - vb * k / q + k * k) / a0,
            ],
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / q + k * k) / a0,
        )
    }

    /// Stage 2 of BS.1770 K-weighting: the revised low-frequency B-curve (a high-pass near 38 Hz).
    pub fn k_highpass(rate: f64) -> Biquad {
        let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
        let k = (PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        Biquad::new(
            [1.0, -2.0, 1.0],
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / q + k * k) / a0,
        )
    }

    /// 2nd-order Butterworth low-pass. Two in a row make a Linkwitz-Riley 4th order.
    pub fn lowpass(rate: f64, fc: f64) -> Biquad {
        let w0 = 2.0 * PI * fc / rate;
        let alpha = w0.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let cosw = w0.cos();
        let a0 = 1.0 + alpha;
        let b = (1.0 - cosw) / 2.0 / a0;
        Biquad::new([b, 2.0 * b, b], -2.0 * cosw / a0, (1.0 - alpha) / a0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gain of a filter at `freq`, measured by running a sine through it.
    fn gain_db(mut stages: Vec<Biquad>, rate: f64, freq: f64) -> f64 {
        let n = (rate * 2.0) as usize;
        let (mut sum_in, mut sum_out) = (0.0, 0.0);
        for i in 0..n {
            let x = (2.0 * PI * freq * i as f64 / rate).sin();
            let mut y = x;
            for s in &mut stages {
                y = s.process(y);
            }
            if i > n / 2 {
                sum_in += x * x;
                sum_out += y * y;
            }
        }
        10.0 * (sum_out / sum_in).log10()
    }

    #[test]
    fn k_weighting_curve_matches_the_standard() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            let k = || vec![Biquad::k_shelf(rate), Biquad::k_highpass(rate)];
            // About +0.7 dB at 1 kHz (the reason for the -0.691 in the LUFS formula), +4 dB up high,
            // the high-pass only biting well below 100 Hz.
            let g1k = gain_db(k(), rate, 1000.0);
            assert!((g1k - 0.691).abs() < 0.05, "{rate}: {g1k}");
            let g10k = gain_db(k(), rate, 10_000.0);
            assert!((g10k - 4.0).abs() < 0.5, "{rate}: {g10k}");
            let g20 = gain_db(k(), rate, 20.0);
            assert!(g20 < -6.0, "{rate}: {g20}");
            let g100 = gain_db(k(), rate, 100.0);
            assert!(g100 > -2.0 && g100 < 0.0, "{rate}: {g100}"); // bass still counts almost fully
        }
    }

    #[test]
    fn linkwitz_riley_halves_sum_flat_at_the_crossover() {
        // Each LR4 half is -6 dB at the crossover frequency.
        let rate = 44_100.0;
        let g = gain_db(
            vec![Biquad::lowpass(rate, 150.0), Biquad::lowpass(rate, 150.0)],
            rate,
            150.0,
        );
        assert!((g + 6.02).abs() < 0.2, "{g}");
        assert!(
            gain_db(
                vec![Biquad::lowpass(rate, 150.0), Biquad::lowpass(rate, 150.0)],
                rate,
                2000.0
            ) < -40.0
        );
    }
}
