//! Audio playback for previewing edit lists, with a choice of output device (including ASIO
//! when built with the `asio` feature) and output channel pair.

pub mod convert;
pub mod cpal_backend;
pub mod device;
pub mod engine;
#[cfg(test)]
mod engine_tests;
pub mod resample;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use cpal_backend::CpalBackend;
pub use device::{Backend, DeviceInfo, DeviceSelection, OpenedInfo};
pub use engine::{Error, LoadInfo, Playback, PlaybackState, Processing};

/// Start the engine on the system's audio devices.
pub fn start_system() -> Playback {
    Playback::start(|| Box::new(CpalBackend::new()))
}
