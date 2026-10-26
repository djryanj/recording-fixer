//! Streaming sample-rate conversion for the playback path.
//!
//! Wraps rubato's FFT resampler (fixed ratio, high quality). It hides the chunking, drops the
//! resampler's start-up delay so the output lines up with the input, and trims the tail so that
//! exactly `ceil(frames_in * ratio)` frames come out.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};

const CHUNK: usize = 1024;

pub struct StreamResampler {
    inner: Fft<f32>,
    channels: usize,
    ratio: f64,
    rate_in: u64,
    rate_out: u64,
    /// Input frames not yet processed (interleaved).
    pending: Vec<f32>,
    scratch: Vec<f32>,
    /// Start-up delay frames still to drop from the output.
    skip: usize,
    frames_in: u64,
    frames_out: u64,
}

impl StreamResampler {
    pub fn new(rate_in: u32, rate_out: u32, channels: usize) -> Result<Self, String> {
        let inner = Fft::<f32>::new(
            rate_in as usize,
            rate_out as usize,
            CHUNK,
            channels,
            FixedSync::Input,
        )
        .map_err(|e| format!("could not create resampler: {e}"))?;
        let skip = inner.output_delay();
        Ok(StreamResampler {
            ratio: f64::from(rate_out) / f64::from(rate_in),
            rate_in: u64::from(rate_in),
            rate_out: u64::from(rate_out),
            channels,
            pending: Vec::new(),
            scratch: vec![0.0; inner.output_frames_max() * channels],
            inner,
            skip,
            frames_in: 0,
            frames_out: 0,
        })
    }

    pub fn ratio(&self) -> f64 {
        self.ratio
    }

    /// Run one chunk through the resampler and append what it produced to `out`.
    fn process(
        &mut self,
        chunk: &[f32],
        partial: Option<usize>,
        out: &mut Vec<f32>,
    ) -> Result<(), String> {
        let need_out = self.inner.output_frames_next();
        if self.scratch.len() < need_out * self.channels {
            self.scratch.resize(need_out * self.channels, 0.0);
        }
        let frames = chunk.len() / self.channels;
        let input =
            InterleavedSlice::new(chunk, self.channels, frames).map_err(|e| e.to_string())?;
        let mut output = InterleavedSlice::new_mut(&mut self.scratch, self.channels, need_out)
            .map_err(|e| e.to_string())?;
        let indexing = Indexing {
            partial_len: partial,
            ..Indexing::default()
        };
        let (_, written) = self
            .inner
            .process_into_buffer(&input, &mut output, Some(&indexing))
            .map_err(|e| e.to_string())?;
        let drop = self.skip.min(written);
        self.skip -= drop;
        out.extend_from_slice(&self.scratch[drop * self.channels..written * self.channels]);
        self.frames_out += (written - drop) as u64;
        Ok(())
    }

    /// Feed interleaved input; resampled frames are appended to `out`.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), String> {
        self.frames_in += (input.len() / self.channels) as u64;
        self.pending.extend_from_slice(input);
        loop {
            let need = self.inner.input_frames_next() * self.channels;
            if self.pending.len() < need {
                return Ok(());
            }
            let chunk: Vec<f32> = self.pending.drain(..need).collect();
            self.process(&chunk, None, out)?;
        }
    }

    /// Flush the tail. After this the total output is exactly `ceil(frames_in * ratio)` frames.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), String> {
        // Exact integer arithmetic: 44100 * (48000 / 44100) is 48000.00000000001 in floating point.
        let expected = (self.frames_in * self.rate_out).div_ceil(self.rate_in);
        if !self.pending.is_empty() {
            let frames = self.pending.len() / self.channels;
            let mut chunk = std::mem::take(&mut self.pending);
            chunk.resize(self.inner.input_frames_next() * self.channels, 0.0);
            self.process(&chunk, Some(frames), out)?;
        }
        let silence = vec![0.0; self.inner.input_frames_next() * self.channels];
        while self.frames_out < expected {
            self.process(&silence, Some(0), out)?;
        }
        // Drop anything beyond the expected length (the flush overshoots by up to a chunk).
        let excess = (self.frames_out - expected) as usize;
        out.truncate(out.len() - excess * self.channels);
        self.frames_out = expected;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, hz: f64, frames: usize, channels: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v =
                    (i as f64 * hz * std::f64::consts::TAU / f64::from(rate)).sin() as f32 * 0.5;
                std::iter::repeat_n(v, channels)
            })
            .collect()
    }

    fn run(rate_in: u32, rate_out: u32, channels: usize, input: &[f32], block: usize) -> Vec<f32> {
        let mut r = StreamResampler::new(rate_in, rate_out, channels).unwrap();
        let mut out = Vec::new();
        for b in input.chunks(block * channels) {
            r.push(b, &mut out).unwrap();
        }
        r.finish(&mut out).unwrap();
        out
    }

    #[test]
    fn output_length_is_exactly_ceil_of_input_times_ratio() {
        for (a, b, frames) in [
            (44100, 48000, 44100usize),
            (48000, 44100, 48000),
            (44100, 96000, 5000),
            (44100, 48000, 1),
            (44100, 48000, 1023),
            (44100, 48000, 1025),
        ] {
            let out = run(a, b, 2, &sine(a, 440.0, frames, 2), 777);
            let expected = (frames as f64 * f64::from(b) / f64::from(a)).ceil() as usize;
            assert_eq!(out.len() / 2, expected, "{a}->{b}, {frames} frames");
        }
    }

    #[test]
    fn a_sine_keeps_its_frequency_phase_and_level() {
        let (a, b, hz) = (44100, 48000, 1000.0);
        let out = run(a, b, 1, &sine(a, hz, 44100, 1), 1000);
        // Compare against the ideal sine at the new rate, away from the very edges.
        let mut worst = 0.0f32;
        for (i, &v) in out.iter().enumerate().take(46000).skip(2000) {
            let ideal = (i as f64 * hz * std::f64::consts::TAU / f64::from(b)).sin() as f32 * 0.5;
            worst = worst.max((v - ideal).abs());
        }
        assert!(worst < 0.01, "worst error {worst}");
    }

    #[test]
    fn block_size_does_not_change_the_result() {
        let input = sine(44100, 523.0, 20000, 2);
        let a = run(44100, 48000, 2, &input, 100);
        let b = run(44100, 48000, 2, &input, 4096);
        assert_eq!(a.len(), b.len());
        let worst = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-6, "{worst}");
    }

    #[test]
    fn channels_stay_independent() {
        let l = sine(44100, 300.0, 8000, 1);
        let r = sine(44100, 900.0, 8000, 1);
        let inter: Vec<f32> = l.iter().zip(&r).flat_map(|(&a, &b)| [a, b]).collect();
        let out = run(44100, 48000, 2, &inter, 512);
        let (ol, or): (Vec<f32>, Vec<f32>) =
            out.as_chunks::<2>().0.iter().map(|f| (f[0], f[1])).unzip();
        let solo_l = run(44100, 48000, 1, &l, 512);
        let solo_r = run(44100, 48000, 1, &r, 512);
        assert!(ol.iter().zip(&solo_l).all(|(a, b)| (a - b).abs() < 1e-5));
        assert!(or.iter().zip(&solo_r).all(|(a, b)| (a - b).abs() < 1e-5));
    }

    #[test]
    fn silence_in_is_silence_out() {
        let out = run(44100, 48000, 2, &vec![0.0; 20000], 1000);
        assert!(out.iter().all(|&v| v.abs() < 1e-9));
    }

    #[test]
    fn no_input_gives_no_output() {
        let mut r = StreamResampler::new(44100, 48000, 2).unwrap();
        let mut out = Vec::new();
        r.finish(&mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn rejects_zero_rates() {
        assert!(StreamResampler::new(0, 48000, 2).is_err());
    }
}
