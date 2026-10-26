//! The playback engine: edit list in, audio device out.
//!
//! ```text
//!  edit list --> decoder thread --> ring buffer --> device callback --> speakers
//!               (read, f32, resample,  (lock-free)   (copy, gain, meter;
//!                route to channel pair)               never blocks or allocates)
//! ```
//!
//! The device stream is opened once and keeps running; "paused" just means the callback outputs
//! silence. Seeking bumps a generation counter; the callback notices, drops everything buffered
//! and acknowledges, and only then does the decoder write audio from the new position, so no
//! stale audio is ever heard.

use crate::convert::{decode_be, route};
use crate::device::{
    AudioCallback, Backend, DeviceInfo, DeviceSelection, OpenedInfo, OpenedStream,
};
use crate::resample::StreamResampler;
use edit_core::{Clip, Plan, PlanReader};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use serde::Serialize;
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::*};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Frames decoded per block.
const BLOCK_FRAMES: usize = 4096;
/// Seconds of audio the ring buffer holds.
const RING_SECONDS: f64 = 1.0;
/// How long the decoder waits for the device callback to acknowledge a seek.
const ACK_TIMEOUT: Duration = Duration::from_millis(1000);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Plan(#[from] edit_core::Error),
    #[error("{0}")]
    Device(String),
    #[error("the playback engine has stopped")]
    Stopped,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize)]
pub struct LoadInfo {
    pub duration_secs: f64,
    pub total_frames: u64,
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PlaybackState {
    pub loaded: bool,
    pub playing: bool,
    /// Playback reached the end of the material.
    pub ended: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    /// Recent peak level of the left and right output, 0.0 to 1.0.
    pub peak_l: f32,
    pub peak_r: f32,
    pub volume: f32,
    /// Times the device ran out of audio (a glitch).
    pub underruns: u64,
    pub device: Option<OpenedInfo>,
    pub error: Option<String>,
}

/// State shared with the device callback. Everything here is lock-free.
struct AudioShared {
    gen: AtomicU64,
    acked: AtomicU64,
    playing: AtomicBool,
    eof: AtomicBool,
    ended: AtomicBool,
    /// Set the moment a seek/reload is requested: the callback outputs silence and consumes
    /// nothing until the decoder has repositioned, so stale audio is never heard.
    hold: AtomicBool,
    frames_played: AtomicU64,
    base_bits: AtomicU64,
    pending_base_bits: AtomicU64,
    peak: [AtomicU32; 2],
    volume: AtomicU32,
    underruns: AtomicU64,
    buffered: AtomicUsize,
    rate: u32,
    channels: usize,
    /// First channel of the output pair, for metering.
    meter_base: usize,
}

impl AudioShared {
    fn new(rate: u32, channels: usize, pair: usize) -> Self {
        AudioShared {
            gen: AtomicU64::new(0),
            acked: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            eof: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            hold: AtomicBool::new(false),
            frames_played: AtomicU64::new(0),
            base_bits: AtomicU64::new(0f64.to_bits()),
            pending_base_bits: AtomicU64::new(0f64.to_bits()),
            peak: [AtomicU32::new(0), AtomicU32::new(0)],
            volume: AtomicU32::new(1.0f32.to_bits()),
            underruns: AtomicU64::new(0),
            buffered: AtomicUsize::new(0),
            rate,
            channels,
            meter_base: if pair * 2 + 1 < channels { pair * 2 } else { 0 },
        }
    }

    fn position(&self) -> f64 {
        f64::from_bits(self.base_bits.load(Relaxed))
            + self.frames_played.load(Relaxed) as f64 / f64::from(self.rate)
    }
}

fn make_callback(mut cons: HeapCons<f32>, sh: Arc<AudioShared>) -> AudioCallback {
    let mut local_gen = 0u64;
    let ch = sh.channels;
    Box::new(move |out: &mut [f32]| {
        let g = sh.gen.load(Acquire);
        if g != local_gen {
            // A seek (or a new edit list): drop everything buffered and say so.
            cons.clear();
            local_gen = g;
            sh.frames_played.store(0, Relaxed);
            sh.base_bits
                .store(sh.pending_base_bits.load(Relaxed), Relaxed);
            sh.buffered.store(0, Relaxed);
            out.fill(0.0);
            sh.acked.store(g, Release);
            return;
        }
        if sh.hold.load(Acquire) || !sh.playing.load(Acquire) {
            out.fill(0.0);
            sh.peak[0].store(0, Relaxed);
            sh.peak[1].store(0, Relaxed);
            sh.buffered.store(cons.occupied_len(), Relaxed);
            return;
        }
        let n = cons.pop_slice(out);
        if n < out.len() {
            out[n..].fill(0.0);
            if sh.eof.load(Acquire) && cons.is_empty() {
                sh.ended.store(true, Release);
                sh.playing.store(false, Release);
            } else {
                sh.underruns.fetch_add(1, Relaxed);
            }
        }
        let vol = f32::from_bits(sh.volume.load(Relaxed));
        if vol != 1.0 {
            out[..n].iter_mut().for_each(|s| *s *= vol);
        }
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for frame in out[..n].chunks_exact(ch) {
            l = l.max(frame[sh.meter_base].abs());
            r = r.max(frame[(sh.meter_base + 1).min(ch - 1)].abs());
        }
        sh.peak[0].store(l.to_bits(), Relaxed);
        sh.peak[1].store(r.to_bits(), Relaxed);
        sh.frames_played.fetch_add((n / ch) as u64, Relaxed);
        sh.buffered.store(cons.occupied_len(), Relaxed);
    })
}

// ---- decoder --------------------------------------------------------------------------

struct DecoderCfg {
    dev_rate: u32,
    dev_ch: usize,
    pair: usize,
}

enum DecCmd {
    Seek(f64),
}

struct Decoder {
    tx: mpsc::Sender<DecCmd>,
    stop: Arc<AtomicBool>,
    join: JoinHandle<HeapProd<f32>>,
}

impl Decoder {
    /// Stop the thread and get the ring buffer's producer back.
    fn stop(self) -> Option<HeapProd<f32>> {
        self.stop.store(true, Release);
        self.join.join().ok()
    }
}

struct Source {
    plan: Arc<Plan>,
    cfg: DecoderCfg,
    reader: PlanReader,
    resampler: Option<StreamResampler>,
    pending: Vec<f32>,
    off: usize,
    done: bool,
    bytes: Vec<u8>,
    decoded: Vec<f32>,
    resampled: Vec<f32>,
}

impl Source {
    fn new(plan: Arc<Plan>, cfg: DecoderCfg) -> Self {
        Source {
            reader: plan.reader_from(0),
            plan,
            cfg,
            resampler: None,
            pending: Vec::new(),
            off: 0,
            done: false,
            bytes: Vec::new(),
            decoded: Vec::new(),
            resampled: Vec::new(),
        }
    }

    fn reset(&mut self, secs: f64) -> std::result::Result<(), String> {
        let src_rate = self.plan.format.sample_rate.round() as u32;
        let frame =
            ((secs.max(0.0) * f64::from(src_rate)).round() as u64).min(self.plan.total_frames);
        self.reader = self.plan.reader_from(frame);
        self.pending.clear();
        self.off = 0;
        self.done = frame >= self.plan.total_frames;
        self.resampler = if src_rate == self.cfg.dev_rate {
            None
        } else {
            Some(StreamResampler::new(
                src_rate,
                self.cfg.dev_rate,
                usize::from(self.plan.format.channels),
            )?)
        };
        Ok(())
    }

    /// Decode, resample and route the next block into `pending`.
    fn fill(&mut self) -> std::result::Result<(), String> {
        let f = &self.plan.format;
        let align = f.block_align() as usize;
        let want = BLOCK_FRAMES * align;
        self.bytes.resize(want, 0);
        let mut got = 0;
        while got < want {
            let n = self
                .reader
                .read(&mut self.bytes[got..])
                .map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            got += n;
        }
        let eof = got < want;
        let whole = got - got % align;
        self.decoded.clear();
        decode_be(&self.bytes[..whole], f.bits, &mut self.decoded);
        self.pending.clear();
        self.off = 0;
        let src_ch = usize::from(f.channels);
        match &mut self.resampler {
            Some(rs) => {
                self.resampled.clear();
                rs.push(&self.decoded, &mut self.resampled)?;
                if eof {
                    rs.finish(&mut self.resampled)?;
                }
                route(
                    &self.resampled,
                    src_ch,
                    self.cfg.dev_ch,
                    self.cfg.pair,
                    &mut self.pending,
                );
            }
            None => route(
                &self.decoded,
                src_ch,
                self.cfg.dev_ch,
                self.cfg.pair,
                &mut self.pending,
            ),
        }
        self.done = eof;
        Ok(())
    }
}

/// Tell the callback to drop its buffer and wait (briefly) for it to acknowledge.
fn begin(
    source: &mut Source,
    sh: &AudioShared,
    stop: &AtomicBool,
    secs: f64,
) -> std::result::Result<(), String> {
    sh.hold.store(true, Release);
    sh.eof.store(false, Release);
    sh.ended.store(false, Release);
    let reset = source.reset(secs);
    // While held the callback is not counting frames, so the position can be set right away
    // (state() then shows the new position at once, not only after the next callback).
    sh.frames_played.store(0, Relaxed);
    sh.base_bits.store(secs.max(0.0).to_bits(), Relaxed);
    sh.pending_base_bits.store(secs.max(0.0).to_bits(), Relaxed);
    let g = sh.gen.fetch_add(1, AcqRel) + 1;
    let started = std::time::Instant::now();
    while sh.acked.load(Acquire) != g && !stop.load(Acquire) && started.elapsed() < ACK_TIMEOUT {
        thread::sleep(Duration::from_millis(1));
    }
    sh.hold.store(false, Release);
    reset
}

fn spawn_decoder(
    plan: Arc<Plan>,
    cfg: DecoderCfg,
    mut prod: HeapProd<f32>,
    sh: Arc<AudioShared>,
    error: Arc<Mutex<Option<String>>>,
    start_secs: f64,
) -> Decoder {
    let (tx, rx) = mpsc::channel::<DecCmd>();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = stop.clone();
    let join = thread::spawn(move || {
        let mut source = Source::new(plan, cfg);
        let fail = |msg: String| {
            if let Ok(mut e) = error.lock() {
                *e = Some(msg);
            }
        };
        if let Err(e) = begin(&mut source, &sh, &stop_t, start_secs) {
            fail(e);
            source.done = true;
        }
        'run: loop {
            if stop_t.load(Acquire) {
                break;
            }
            loop {
                match rx.try_recv() {
                    Ok(DecCmd::Seek(secs)) => {
                        if let Err(e) = begin(&mut source, &sh, &stop_t, secs) {
                            fail(e);
                            source.done = true;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => break 'run,
                }
            }
            if source.off >= source.pending.len() {
                if source.done {
                    sh.eof.store(true, Release);
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                if let Err(e) = source.fill() {
                    fail(e);
                    source.done = true;
                    continue;
                }
            }
            let n = prod.push_slice(&source.pending[source.off..]);
            source.off += n;
            if n == 0 {
                thread::sleep(Duration::from_millis(2));
            }
        }
        prod
    });
    Decoder { tx, stop, join }
}

// ---- the engine thread and its public handle -------------------------------------------

enum Cmd {
    Devices(mpsc::Sender<Vec<DeviceInfo>>),
    Details(String, mpsc::Sender<Option<DeviceInfo>>),
    SetDevice(DeviceSelection, mpsc::Sender<Result<OpenedInfo>>),
    Load {
        clips: Vec<Clip>,
        start_secs: f64,
        reply: mpsc::Sender<Result<LoadInfo>>,
    },
    Play,
    Pause,
    Seek(f64),
    Volume(f32),
    Shutdown,
}

#[derive(Default)]
struct Common {
    shared: Mutex<Option<Arc<AudioShared>>>,
    device: Mutex<Option<OpenedInfo>>,
    duration: Mutex<Option<f64>>,
    error: Arc<Mutex<Option<String>>>,
    volume: Mutex<f32>,
}

/// Handle to the playback engine. Cheap to share; all methods are safe to call from any thread.
pub struct Playback {
    tx: mpsc::Sender<Cmd>,
    common: Arc<Common>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

struct Active {
    _keepalive: Box<dyn std::any::Any>,
    info: OpenedInfo,
    shared: Arc<AudioShared>,
    /// The ring's producer while no decoder holds it.
    prod: Option<HeapProd<f32>>,
}

struct EngineThread {
    backend: Box<dyn Backend>,
    rx: mpsc::Receiver<Cmd>,
    common: Arc<Common>,
    selection: DeviceSelection,
    active: Option<Active>,
    plan: Option<Arc<Plan>>,
    decoder: Option<Decoder>,
    volume: f32,
}

impl EngineThread {
    fn run(mut self) {
        while let Ok(cmd) = self.rx.recv() {
            match cmd {
                Cmd::Devices(reply) => {
                    let _ = reply.send(self.backend.devices());
                }
                Cmd::Details(id, reply) => {
                    let _ = reply.send(self.backend.details(&id));
                }
                Cmd::SetDevice(sel, reply) => {
                    let _ = reply.send(self.set_device(sel));
                }
                Cmd::Load {
                    clips,
                    start_secs,
                    reply,
                } => {
                    let _ = reply.send(self.load(&clips, start_secs));
                }
                Cmd::Play => self.play(),
                Cmd::Pause => {
                    if let Some(a) = &self.active {
                        a.shared.playing.store(false, Release);
                    }
                }
                Cmd::Seek(secs) => self.seek(secs),
                Cmd::Volume(v) => {
                    self.volume = v.clamp(0.0, 1.5);
                    if let Ok(mut c) = self.common.volume.lock() {
                        *c = self.volume;
                    }
                    if let Some(a) = &self.active {
                        a.shared.volume.store(self.volume.to_bits(), Relaxed);
                    }
                }
                Cmd::Shutdown => break,
            }
        }
        self.stop_decoder();
    }

    fn stop_decoder(&mut self) {
        if let (Some(d), Some(a)) = (self.decoder.take(), self.active.as_mut()) {
            // Mute right now: the old decoder is going away and whatever it buffered is stale.
            a.shared.hold.store(true, Release);
            a.prod = d.stop();
        }
    }

    fn seek(&mut self, secs: f64) {
        if let (Some(d), Some(a)) = (&self.decoder, &self.active) {
            // Mute immediately; the decoder lifts this once it has repositioned.
            a.shared.hold.store(true, Release);
            let _ = d.tx.send(DecCmd::Seek(secs));
        }
    }

    fn position(&self) -> f64 {
        self.active.as_ref().map_or(0.0, |a| a.shared.position())
    }

    fn open(&mut self, preferred_rate: u32) -> Result<()> {
        self.stop_decoder();
        self.active = None; // closes the previous device first
        if let Ok(mut s) = self.common.shared.lock() {
            *s = None;
        }
        // The callback needs the ring buffer, whose size depends on the channel count the device
        // actually opened with. So the device gets a forwarding callback that outputs silence
        // until the real one is installed.
        let late: Arc<Mutex<Option<AudioCallback>>> = Arc::new(Mutex::new(None));
        let late_cb = late.clone();
        let forwarding: AudioCallback = Box::new(move |out: &mut [f32]| match late_cb.try_lock() {
            Ok(mut g) => match g.as_mut() {
                Some(cb) => cb(out),
                None => out.fill(0.0),
            },
            Err(_) => out.fill(0.0),
        });
        let opened: OpenedStream = self
            .backend
            .open(&self.selection, preferred_rate, forwarding)
            .map_err(Error::Device)?;
        let info = opened.info.clone();
        let channels = usize::from(info.channels.max(1));
        let shared = Arc::new(AudioShared::new(info.rate, channels, info.out_pair));
        shared.volume.store(self.volume.to_bits(), Relaxed);
        let capacity =
            ((f64::from(info.rate) * RING_SECONDS) as usize * channels).max(channels * 1024);
        let (prod, cons) = HeapRb::<f32>::new(capacity).split();
        if let Ok(mut g) = late.lock() {
            *g = Some(make_callback(cons, shared.clone()));
        }
        if let Ok(mut s) = self.common.shared.lock() {
            *s = Some(shared.clone());
        }
        if let Ok(mut d) = self.common.device.lock() {
            *d = Some(info.clone());
        }
        self.active = Some(Active {
            _keepalive: opened.keepalive,
            info,
            shared,
            prod: Some(prod),
        });
        Ok(())
    }

    fn start_decoder(&mut self, start_secs: f64) -> Result<()> {
        let (Some(plan), Some(active)) = (self.plan.clone(), self.active.as_mut()) else {
            return Ok(());
        };
        let prod = active.prod.take().ok_or(Error::Stopped)?;
        // Report the new position straight away; the decoder thread starts a moment later.
        let sh = &active.shared;
        sh.hold.store(true, Release);
        sh.frames_played.store(0, Relaxed);
        sh.base_bits.store(start_secs.max(0.0).to_bits(), Relaxed);
        sh.pending_base_bits
            .store(start_secs.max(0.0).to_bits(), Relaxed);
        let cfg = DecoderCfg {
            dev_rate: active.info.rate,
            dev_ch: usize::from(active.info.channels.max(1)),
            pair: active.info.out_pair,
        };
        self.decoder = Some(spawn_decoder(
            plan,
            cfg,
            prod,
            active.shared.clone(),
            self.common.error.clone(),
            start_secs,
        ));
        Ok(())
    }

    fn set_device(&mut self, sel: DeviceSelection) -> Result<OpenedInfo> {
        let resume = self.position();
        let was_playing = self
            .active
            .as_ref()
            .is_some_and(|a| a.shared.playing.load(Acquire));
        self.selection = sel;
        let rate = self
            .plan
            .as_ref()
            .map_or(44100, |p| p.format.sample_rate.round() as u32);
        self.open(rate)?;
        self.start_decoder(resume)?;
        if was_playing {
            self.play();
        }
        self.active
            .as_ref()
            .map(|a| a.info.clone())
            .ok_or(Error::Stopped)
    }

    fn load(&mut self, clips: &[Clip], start_secs: f64) -> Result<LoadInfo> {
        let plan = Arc::new(edit_core::plan(clips)?);
        let rate = plan.format.sample_rate.round() as u32;
        if let Ok(mut e) = self.common.error.lock() {
            *e = None;
        }
        if self.active.is_none() {
            self.open(rate)?;
        }
        self.stop_decoder();
        let info = LoadInfo {
            duration_secs: plan.duration_secs(),
            total_frames: plan.total_frames,
            rate,
            channels: plan.format.channels,
            bits: plan.format.bits,
        };
        self.plan = Some(plan);
        if let Ok(mut d) = self.common.duration.lock() {
            *d = Some(info.duration_secs);
        }
        self.start_decoder(start_secs.clamp(0.0, info.duration_secs))?;
        Ok(info)
    }

    fn play(&mut self) {
        let Some(a) = &self.active else { return };
        if self.plan.is_none() {
            return;
        }
        let ended = a.shared.ended.load(Acquire);
        a.shared.playing.store(true, Release);
        if ended {
            self.seek(0.0);
        }
    }
}

impl Playback {
    /// Start the engine thread. `backend` is built on that thread (audio objects are often not `Send`).
    pub fn start<F>(backend: F) -> Playback
    where
        F: FnOnce() -> Box<dyn Backend> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let common = Arc::new(Common {
            volume: Mutex::new(1.0),
            ..Default::default()
        });
        let common_t = common.clone();
        let thread = thread::Builder::new()
            .name("playback-engine".into())
            .spawn(move || {
                EngineThread {
                    backend: backend(),
                    rx,
                    common: common_t,
                    selection: DeviceSelection::default(),
                    active: None,
                    plan: None,
                    decoder: None,
                    volume: 1.0,
                }
                .run()
            })
            .expect("spawn playback engine");
        Playback {
            tx,
            common,
            thread: Mutex::new(Some(thread)),
        }
    }

    fn ask<T>(&self, make: impl FnOnce(mpsc::Sender<T>) -> Cmd) -> Result<T> {
        let (reply, rx) = mpsc::channel();
        self.tx.send(make(reply)).map_err(|_| Error::Stopped)?;
        rx.recv().map_err(|_| Error::Stopped)
    }

    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.ask(Cmd::Devices).unwrap_or_default()
    }

    /// Channel count and default rate for one device (may load its driver).
    pub fn device_details(&self, id: &str) -> Option<DeviceInfo> {
        self.ask(|r| Cmd::Details(id.to_string(), r)).ok().flatten()
    }

    /// Choose the output device (and channel pair). Reopens the device, keeping the position.
    pub fn set_device(&self, selection: DeviceSelection) -> Result<OpenedInfo> {
        self.ask(|r| Cmd::SetDevice(selection, r))?
    }

    /// Replace the material. Playback continues from `start_secs` of the new edit list.
    pub fn load(&self, clips: Vec<Clip>, start_secs: f64) -> Result<LoadInfo> {
        self.ask(|reply| Cmd::Load {
            clips,
            start_secs,
            reply,
        })?
    }

    pub fn play(&self) {
        let _ = self.tx.send(Cmd::Play);
    }
    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn seek(&self, secs: f64) {
        let _ = self.tx.send(Cmd::Seek(secs));
    }
    pub fn set_volume(&self, volume: f32) {
        let _ = self.tx.send(Cmd::Volume(volume));
    }

    pub fn state(&self) -> PlaybackState {
        let shared = self.common.shared.lock().ok().and_then(|s| s.clone());
        let duration = self
            .common
            .duration
            .lock()
            .ok()
            .and_then(|d| *d)
            .unwrap_or(0.0);
        let mut st = PlaybackState {
            loaded: duration > 0.0,
            duration_secs: duration,
            volume: self.common.volume.lock().map_or(1.0, |v| *v),
            device: self.common.device.lock().ok().and_then(|d| d.clone()),
            error: self.common.error.lock().ok().and_then(|e| e.clone()),
            ..Default::default()
        };
        if let Some(sh) = shared {
            st.ended = sh.ended.load(Acquire);
            st.playing = sh.playing.load(Acquire);
            st.position_secs = if st.ended {
                duration
            } else {
                sh.position().min(duration)
            };
            st.peak_l = f32::from_bits(sh.peak[0].load(Relaxed));
            st.peak_r = f32::from_bits(sh.peak[1].load(Relaxed));
            st.underruns = sh.underruns.load(Relaxed);
        }
        st
    }

    /// Frames waiting in the ring buffer (tests use this to let the decoder get ahead).
    pub fn buffered_frames(&self) -> usize {
        let sh = self.common.shared.lock().ok().and_then(|s| s.clone());
        sh.map_or(0, |s| s.buffered.load(Relaxed) / s.channels.max(1))
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(t) = self.thread.lock().ok().and_then(|mut t| t.take()) {
            let _ = t.join();
        }
    }
}
