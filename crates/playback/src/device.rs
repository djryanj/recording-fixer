//! What the engine needs from an audio output, so it can be tested without hardware.

use serde::{Deserialize, Serialize};
use std::any::Any;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeviceInfo {
    /// Stable identifier ("host:device"), safe to store and pass back in a [`DeviceSelection`].
    pub id: String,
    /// Audio API: "wasapi", "asio", "coreaudio", ...
    pub host: String,
    pub name: String,
    /// Most output channels the device offers.
    pub channels: u16,
    pub default_rate: u32,
    pub is_default: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceSelection {
    /// `None` means the system default output.
    pub id: Option<String>,
    /// Which pair of output channels carries the audio: 0 = channels 1-2, 1 = 3-4, ...
    pub out_pair: usize,
    /// Requested buffer size in frames (smaller is lower latency); `None` for the driver default.
    pub buffer_frames: Option<u32>,
}

/// What a device was actually opened as.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenedInfo {
    pub id: String,
    pub host: String,
    pub name: String,
    pub rate: u32,
    pub channels: u16,
    pub out_pair: usize,
    pub buffer_frames: Option<u32>,
}

/// Fills interleaved `f32` frames for the opened device. Must not block or allocate.
pub type AudioCallback = Box<dyn FnMut(&mut [f32]) + Send>;

/// An open output. Dropping `keepalive` closes the device.
pub struct OpenedStream {
    pub info: OpenedInfo,
    pub keepalive: Box<dyn Any>,
}

pub trait Backend {
    fn devices(&self) -> Vec<DeviceInfo>;
    /// Full details (channel count, default rate) for one device. Listing can skip these for
    /// drivers that are expensive to query (ASIO), so the UI asks when a device is chosen.
    fn details(&self, _id: &str) -> Option<DeviceInfo> {
        None
    }
    /// Open `selection`, preferring `preferred_rate` if the device supports it. The callback is
    /// driven continuously by the device until the returned stream is dropped.
    fn open(
        &self,
        selection: &DeviceSelection,
        preferred_rate: u32,
        callback: AudioCallback,
    ) -> Result<OpenedStream, String>;
}
