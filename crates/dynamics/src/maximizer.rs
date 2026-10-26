//! The maximizer as a stage in the export pipeline: PCM in, PCM out, same format.
//!
//! `prepare` measures the programme (one streaming read), plans the leveling curve, and optionally
//! rehearses the whole chain with the output thrown away to find out how far the limiter pulled the
//! loudness below the target, then corrects for it. `Maximizer` then processes the real export as a
//! `Read`, so it slots in front of the AIFF writer or an encoder the way the fades do.
//!
//! The output has the same sample format as the input: audio that arrives as 16-bit leaves as 16-bit
//! (with dither), 24-bit as 24-bit. Nothing is padded out to a bigger format.

use crate::leveler::{self, GainCurve, Leveling};
use crate::limiter::{Limiter, LimiterConfig, LimiterStats};
use crate::loudness::{Analyzer, Profile};
use crate::pcm::{decode, PcmFormat, Quantizer};
use std::io::{self, Read};
use std::sync::{Arc, Mutex};

/// Bytes read from the source at a time (rounded down to whole frames).
const CHUNK: usize = 256 * 1024;
/// A rehearsal is repeated while the result is further than this from the target, in LU.
const CLOSE_ENOUGH_LU: f64 = 0.15;
const MAX_REHEARSALS: usize = 2;
/// Most the rehearsal corrections may add on top of the planned gain, in dB.
const MAX_CORRECTION_DB: f64 = 6.0;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Bring the whole programme to this integrated loudness (LUFS), levelling as configured.
    Loudness(f64),
    /// Raise or lower the whole programme so its highest true peak lands on the ceiling.
    Peak,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub mode: Mode,
    /// True-peak ceiling in dBTP.
    pub ceiling_db: f64,
    pub leveling: Leveling,
    /// Rehearse the chain first and correct the gain so the finished result lands on the target.
    /// Costs one more read of the programme per rehearsal; ignored for `Mode::Peak`.
    pub precise: bool,
}

/// What a stretch of audio measures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measure {
    pub lufs: f64,
    pub range_lu: f64,
    pub true_peak_db: f64,
}

impl Measure {
    fn of(p: &Profile) -> Measure {
        Measure {
            lufs: p.integrated(),
            range_lu: p.loudness_range(),
            true_peak_db: 20.0 * f64::from(p.peak_max()).max(1e-9).log10(),
        }
    }
}

/// What happened, for showing after an export.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Report {
    pub input: Measure,
    pub output: Measure,
    pub limiter: LimiterStats,
    pub target_lufs: Option<f64>,
    pub ceiling_db: f64,
    /// The one static gain and the lowest/highest total gain from the leveller, dB.
    pub static_gain_db: f64,
    pub gain_min_db: f64,
    pub gain_max_db: f64,
}

/// Everything decided before the real pass.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub fmt: PcmFormat,
    pub total_frames: u64,
    pub curve: GainCurve,
    pub ceiling_db: f64,
    pub target_lufs: Option<f64>,
    pub input: Measure,
    /// What the rehearsal predicted, if one was made.
    pub rehearsed: Option<Report>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Measuring,
    Rehearsing,
}

pub type Progress<'a> = &'a mut dyn FnMut(Phase, u64, u64) -> bool;

fn read_chunks(
    mut reader: impl Read,
    fmt: PcmFormat,
    total_frames: u64,
    phase: Phase,
    progress: Progress,
    mut each: impl FnMut(&[f32]),
) -> Result<(), Error> {
    let fb = fmt.frame_bytes();
    let total = total_frames * fb as u64;
    let mut bytes = vec![0u8; CHUNK / fb * fb];
    let mut samples = Vec::with_capacity(bytes.len());
    let mut done = 0u64;
    while done < total {
        let want = (total - done).min(bytes.len() as u64) as usize;
        reader.read_exact(&mut bytes[..want])?;
        samples.clear();
        decode(fmt, &bytes[..want], &mut samples);
        each(&samples);
        done += want as u64;
        if !progress(phase, done, total) {
            return Err(Error::Cancelled);
        }
    }
    Ok(())
}

/// One streaming pass: the loudness profile of the programme.
pub fn measure(
    reader: impl Read,
    fmt: PcmFormat,
    total_frames: u64,
    progress: Progress,
) -> Result<Profile, Error> {
    let mut an = Analyzer::new(fmt.rate, usize::from(fmt.channels), false);
    read_chunks(reader, fmt, total_frames, Phase::Measuring, progress, |s| {
        an.push(s)
    })?;
    Ok(an.finish())
}

/// Run the chain over the programme and discard the audio, to see what it would produce.
fn rehearse(reader: impl Read, prepared: &Prepared, progress: Progress) -> Result<Report, Error> {
    let mut m = Maximizer::new(reader, prepared);
    let handle = m.report_handle();
    let mut buf = vec![0u8; CHUNK];
    let total = prepared.total_frames * prepared.fmt.frame_bytes() as u64;
    let mut done = 0u64;
    loop {
        let n = m.read(&mut buf)?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if !progress(Phase::Rehearsing, done.min(total), total) {
            return Err(Error::Cancelled);
        }
    }
    let report = handle
        .lock()
        .unwrap()
        .take()
        .expect("report is written at the end of the stream");
    Ok(report)
}

/// Measure, plan, and (if asked) rehearse. `open` must give a fresh reader over the same PCM each time.
pub fn prepare<R: Read>(
    mut open: impl FnMut() -> io::Result<R>,
    fmt: PcmFormat,
    total_frames: u64,
    params: &Params,
    progress: Progress,
) -> Result<Prepared, Error> {
    let profile = measure(open()?, fmt, total_frames, progress)?;
    let input = Measure::of(&profile);
    let (mut curve, target) = match params.mode {
        Mode::Loudness(t) => (leveler::plan(&profile, t, &params.leveling), Some(t)),
        Mode::Peak => (leveler::normalize_peak(&profile, params.ceiling_db), None),
    };
    let mut prepared = Prepared {
        fmt,
        total_frames,
        curve: curve.clone(),
        ceiling_db: params.ceiling_db,
        target_lufs: target,
        input,
        rehearsed: None,
    };
    let Some(target) = target.filter(|_| params.precise) else {
        return Ok(prepared);
    };
    if !input.lufs.is_finite() {
        return Ok(prepared);
    }
    let mut added = 0.0;
    for _ in 0..MAX_REHEARSALS {
        let report = rehearse(open()?, &prepared, progress)?;
        prepared.rehearsed = Some(report);
        let error = target - report.output.lufs;
        if !error.is_finite() || error.abs() <= CLOSE_ENOUGH_LU {
            break;
        }
        let step = error.clamp(-MAX_CORRECTION_DB - added, MAX_CORRECTION_DB - added);
        if step.abs() < 0.01 {
            break;
        }
        added += step;
        let k = 10f64.powf(step / 20.0) as f32;
        for g in &mut curve.gain {
            *g *= k;
        }
        curve.static_db += step;
        prepared.curve = curve.clone();
    }
    Ok(prepared)
}

/// The processing stage. Reads PCM from `inner`, writes PCM of the same format.
pub struct Maximizer<R> {
    inner: R,
    fmt: PcmFormat,
    total_frames: u64,
    curve: GainCurve,
    limiter: Limiter,
    quantizer: Quantizer,
    out_analyzer: Option<Analyzer>,
    input: Measure,
    target_lufs: Option<f64>,
    ceiling_db: f64,
    frame: u64,
    carry: Vec<u8>,
    read_buf: Vec<u8>,
    samples: Vec<f32>,
    gains: Vec<f32>,
    limited: Vec<f32>,
    stored: Vec<f32>,
    out: Vec<u8>,
    out_pos: usize,
    done: bool,
    report: Arc<Mutex<Option<Report>>>,
}

impl<R: Read> Maximizer<R> {
    pub fn new(inner: R, prepared: &Prepared) -> Maximizer<R> {
        let fmt = prepared.fmt;
        let channels = usize::from(fmt.channels);
        Maximizer {
            inner,
            fmt,
            total_frames: prepared.total_frames,
            curve: prepared.curve.clone(),
            limiter: Limiter::new(fmt.rate, channels, LimiterConfig::new(prepared.ceiling_db)),
            quantizer: Quantizer::new(fmt),
            out_analyzer: Some(Analyzer::new(fmt.rate, channels, false)),
            input: prepared.input,
            target_lufs: prepared.target_lufs,
            ceiling_db: prepared.ceiling_db,
            frame: 0,
            carry: Vec::new(),
            read_buf: vec![0; CHUNK / fmt.frame_bytes() * fmt.frame_bytes()],
            samples: Vec::new(),
            gains: Vec::new(),
            limited: Vec::new(),
            stored: Vec::new(),
            out: Vec::new(),
            out_pos: 0,
            done: false,
            report: Arc::new(Mutex::new(None)),
        }
    }

    /// Filled in when the stream has been read to its end.
    pub fn report_handle(&self) -> Arc<Mutex<Option<Report>>> {
        self.report.clone()
    }

    fn emit(&mut self) {
        self.out.clear();
        self.out_pos = 0;
        self.stored.clear();
        self.quantizer
            .encode(&self.limited, &mut self.out, &mut self.stored);
        if let Some(an) = &mut self.out_analyzer {
            an.push(&self.stored);
        }
        self.limited.clear();
    }

    fn finish(&mut self) {
        self.limiter.finish(&mut self.limited);
        self.emit();
        let profile = self.out_analyzer.take().expect("finished once").finish();
        *self.report.lock().unwrap() = Some(Report {
            input: self.input,
            output: Measure::of(&profile),
            limiter: self.limiter.stats(),
            target_lufs: self.target_lufs,
            ceiling_db: self.ceiling_db,
            static_gain_db: self.curve.static_db,
            gain_min_db: self.curve.min_db(),
            gain_max_db: self.curve.max_db(),
        });
        self.done = true;
    }

    /// Process the next chunk of the source into `self.out`. Leaves it empty only at the very end.
    fn refill(&mut self) -> io::Result<()> {
        self.out.clear();
        self.out_pos = 0;
        let fb = self.fmt.frame_bytes();
        while self.out.is_empty() && !self.done {
            let remaining = (self.total_frames - self.frame) * fb as u64 - self.carry.len() as u64;
            let want = (self.read_buf.len() as u64).min(remaining) as usize;
            let n = if want == 0 {
                0
            } else {
                self.inner.read(&mut self.read_buf[..want])?
            };
            if n == 0 {
                if self.carry.is_empty() && self.frame >= self.total_frames {
                    self.finish();
                    return Ok(());
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "source ended early",
                ));
            }
            self.carry.extend_from_slice(&self.read_buf[..n]);
            let whole = self.carry.len() / fb * fb;
            if whole == 0 {
                continue;
            }
            self.samples.clear();
            decode(self.fmt, &self.carry[..whole], &mut self.samples);
            self.carry.drain(..whole);
            let frames = whole / fb;
            self.gains.resize(frames, 1.0);
            self.curve.fill(self.frame, &mut self.gains);
            let ch = usize::from(self.fmt.channels);
            for (frame, g) in self.samples.chunks_exact_mut(ch).zip(&self.gains) {
                for s in frame {
                    *s *= *g;
                }
            }
            self.frame += frames as u64;
            self.limiter.process(&self.samples, &mut self.limited);
            self.emit();
        }
        Ok(())
    }
}

impl<R: Read> Read for Maximizer<R> {
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
