//! Export PCM to mp3/aac by piping it into an ffmpeg subprocess.
//!
//! ffmpeg is never linked: it is found at runtime (user setting, bundled copy, or PATH),
//! and audio is streamed to its stdin so no temporary copy of the recording is made.
//! On Windows, Apple's AAC encoder (CoreAudioToolbox.dll, if installed) can encode instead;
//! ffmpeg then only muxes the result into an .m4a.

pub mod adts;
pub mod apple;
#[cfg(windows)]
pub mod apple_aac;
pub mod encoders;
pub mod ffmpeg;
pub mod options;

pub use encoders::{parse_encoders, Encoders};
pub use ffmpeg::{build_args, locate_ffmpeg, probe_encoders, run, version, ExportSpec};
pub use options::{AacMode, Codec, Metadata, Mp3Mode, Mp3Options, APPLE_ENCODER};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ffmpeg was not found; install it or choose its location in settings")]
    FfmpegNotFound,
    #[error("this ffmpeg build has no {0} encoder")]
    MissingEncoder(String),
    #[error("output file already exists: {0}")]
    OutputExists(String),
    #[error("ffmpeg failed ({status}): {log}")]
    Ffmpeg { status: String, log: String },
    #[error("Apple AAC encoder: {0}")]
    Apple(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cancelled")]
    Cancelled,
}

impl From<edit_core::Error> for Error {
    fn from(e: edit_core::Error) -> Self {
        match e {
            edit_core::Error::Cancelled => Error::Cancelled,
            edit_core::Error::Io(io) => Error::Io(io),
            other => Error::Io(std::io::Error::other(other.to_string())),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
