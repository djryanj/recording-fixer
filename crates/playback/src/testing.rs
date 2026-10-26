//! A simulated audio device for tests: a thread calls the engine's callback at roughly real-time
//! pace and records everything it outputs. No audio hardware is touched.

use crate::device::{
    AudioCallback, Backend, DeviceInfo, DeviceSelection, OpenedInfo, OpenedStream,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Everything the simulated device has been asked to play.
#[derive(Clone, Default)]
pub struct SimOutput {
    chunks: Arc<Mutex<Vec<Vec<f32>>>>,
    opened: Arc<Mutex<Vec<OpenedInfo>>>,
}

impl SimOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Interleaved samples from every callback chunk that contained any sound. Chunks of pure
    /// silence (paused, flushing) are dropped, so this is "what you would have heard".
    pub fn audible(&self) -> Vec<f32> {
        self.chunks
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.iter().any(|&s| s != 0.0))
            .flatten()
            .copied()
            .collect()
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.lock().unwrap().len()
    }

    pub fn clear(&self) {
        self.chunks.lock().unwrap().clear();
    }

    /// Every device configuration the engine opened, in order.
    pub fn opened(&self) -> Vec<OpenedInfo> {
        self.opened.lock().unwrap().clone()
    }
}

/// How the simulated device behaves.
#[derive(Clone)]
pub struct SimConfig {
    pub devices: Vec<DeviceInfo>,
    /// Rate the device runs at regardless of what is asked for (forces resampling when it differs).
    pub rate: Option<u32>,
    pub channels: u16,
    pub frames_per_callback: usize,
    pub callback_interval: Duration,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            devices: vec![
                DeviceInfo {
                    id: "sim:main".into(),
                    host: "sim".into(),
                    name: "Simulated Output".into(),
                    channels: 2,
                    default_rate: 44100,
                    is_default: true,
                },
                DeviceInfo {
                    id: "sim:multi".into(),
                    host: "sim".into(),
                    name: "Simulated 8-channel interface".into(),
                    channels: 8,
                    default_rate: 48000,
                    is_default: false,
                },
            ],
            rate: None,
            channels: 2,
            frames_per_callback: 512,
            callback_interval: Duration::from_millis(5),
        }
    }
}

pub struct SimBackend {
    out: SimOutput,
    config: SimConfig,
}

impl SimBackend {
    pub fn new(out: SimOutput, config: SimConfig) -> Self {
        SimBackend { out, config }
    }
}

struct Pump {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Backend for SimBackend {
    fn devices(&self) -> Vec<DeviceInfo> {
        self.config.devices.clone()
    }

    fn details(&self, id: &str) -> Option<DeviceInfo> {
        self.config.devices.iter().find(|d| d.id == id).cloned()
    }

    fn open(
        &self,
        selection: &DeviceSelection,
        preferred_rate: u32,
        mut callback: AudioCallback,
    ) -> Result<OpenedStream, String> {
        let device = match &selection.id {
            Some(id) => self
                .config
                .devices
                .iter()
                .find(|d| &d.id == id)
                .ok_or_else(|| format!("no such device {id}"))?,
            None => self
                .config
                .devices
                .iter()
                .find(|d| d.is_default)
                .or(self.config.devices.first())
                .ok_or("no devices")?,
        };
        let channels = device.channels.max(1);
        let rate = self.config.rate.unwrap_or(preferred_rate);
        let info = OpenedInfo {
            id: device.id.clone(),
            host: device.host.clone(),
            name: device.name.clone(),
            rate,
            channels,
            out_pair: selection.out_pair,
            buffer_frames: Some(self.config.frames_per_callback as u32),
        };
        self.out.opened.lock().unwrap().push(info.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let (stop_t, out, frames, interval) = (
            stop.clone(),
            self.out.chunks.clone(),
            self.config.frames_per_callback,
            self.config.callback_interval,
        );
        let thread = thread::spawn(move || {
            let mut buf = vec![0.0f32; frames * usize::from(channels)];
            while !stop_t.load(Ordering::Acquire) {
                callback(&mut buf);
                out.lock().unwrap().push(buf.clone());
                thread::sleep(interval);
            }
        });
        Ok(OpenedStream {
            info,
            keepalive: Box::new(Pump {
                stop,
                thread: Some(thread),
            }),
        })
    }
}
