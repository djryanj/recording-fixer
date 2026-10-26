//! Parsing, diagnosing and repairing the AIFF files Serato DJ records.
//!
//! Everything here streams: only headers and the trailing ID3 chunk are read to
//! diagnose a file, and repairs copy audio through a fixed-size buffer.

pub mod ext80;
#[cfg(any(test, feature = "testutil"))]
pub mod fixtures;
pub mod header;
pub mod id3;
pub mod inuse;
pub mod parse;
pub mod repair;

pub use parse::{diagnose, diagnose_path, Defect, Diagnosis, Fixed, Format, Health};
pub use repair::{repair_copy, repair_in_place};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not an AIFF file")]
    NotAiff,
    #[error("unsupported AIFF variant: {0}")]
    Unsupported(String),
    #[error("malformed AIFF: {0}")]
    Malformed(&'static str),
    #[error("no COMM chunk found")]
    NoComm,
    #[error("no SSND chunk found")]
    NoSsnd,
    #[error("result is larger than AIFF's 4 GB chunk limit")]
    TooLarge,
    #[error("file does not need repair")]
    NotBroken,
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Progress callback: `(bytes_done, bytes_total)`. Return `false` to cancel.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64) -> bool;
