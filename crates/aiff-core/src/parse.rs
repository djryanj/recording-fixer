//! Diagnose an AIFF file by reading only its chunk headers and the ID3 trailer.

use crate::id3::{self, Id3Info};
use crate::{ext80, Error, Result};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Format {
    pub channels: u16,
    pub bits: u16,
    pub sample_rate: f64,
}

impl Format {
    pub fn bytes_per_sample(&self) -> u64 {
        u64::from(self.bits.div_ceil(8))
    }
    pub fn block_align(&self) -> u64 {
        u64::from(self.channels) * self.bytes_per_sample()
    }
}

/// One way a header disagrees with the audio actually in the file.
#[derive(Debug, Clone, PartialEq)]
pub enum Defect {
    FormSize {
        found: u64,
        expected: u64,
    },
    FrameCount {
        found: u64,
        expected: u64,
    },
    SsndSize {
        found: u64,
        expected: u64,
    },
    /// Trailing bytes after the last whole frame (an interrupted write).
    PartialFrame {
        bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Health {
    Healthy,
    Broken(Vec<Defect>),
}

/// The values a repaired file's header must carry.
#[derive(Debug, Clone, PartialEq)]
pub struct Fixed {
    pub form_size: u64,
    pub ssnd_size: u64,
    pub frames: u64,
    /// Length of the repaired file (partial frames dropped, pad byte added).
    pub file_len: u64,
}

#[derive(Debug, Clone)]
pub struct Diagnosis {
    pub format: Format,
    pub file_len: u64,
    pub comm_pos: u64,
    pub ssnd_pos: u64,
    /// File offset of the first sample.
    pub audio_start: u64,
    /// Whole-frame audio bytes, honouring the header when it is sound.
    pub audio_bytes: u64,
    pub frames: u64,
    /// True when the header was unusable and the audio was assumed to run to EOF.
    pub extent_inferred: bool,
    pub health: Health,
    pub tags: Option<Id3Info>,
    pub fixed: Fixed,
}

impl Diagnosis {
    pub fn duration_secs(&self) -> f64 {
        if self.format.sample_rate > 0.0 {
            self.frames as f64 / self.format.sample_rate
        } else {
            0.0
        }
    }
    pub fn is_healthy(&self) -> bool {
        self.health == Health::Healthy
    }
}

pub fn diagnose_path(path: &Path) -> Result<Diagnosis> {
    let f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    diagnose(std::io::BufReader::with_capacity(8192, f), len)
}

/// Largest ID3 chunk we are willing to read into memory.
const MAX_ID3_BYTES: u64 = 1 << 20;

fn be32(b: &[u8]) -> u64 {
    u64::from(u32::from_be_bytes(b[..4].try_into().unwrap()))
}

struct Comm {
    pos: u64,
    channels: u16,
    frames: u64,
    bits: u16,
    rate: f64,
}

struct Ssnd {
    pos: u64,
    size: u64,
    offset: u64,
}

pub fn diagnose<R: Read + Seek>(mut r: R, file_len: u64) -> Result<Diagnosis> {
    let mut head = [0u8; 12];
    r.read_exact(&mut head).map_err(|_| Error::NotAiff)?;
    if &head[0..4] != b"FORM" {
        return Err(Error::NotAiff);
    }
    match &head[8..12] {
        b"AIFF" => {}
        b"AIFC" => return Err(Error::Unsupported("AIFF-C".into())),
        _ => return Err(Error::NotAiff),
    }
    let form_size = be32(&head[4..8]);

    let mut comm = None;
    let mut ssnd = None;
    let mut tags = None;
    let mut extent_inferred = false;
    let mut pos = 12u64;
    while pos + 8 <= file_len {
        r.seek(SeekFrom::Start(pos))?;
        let mut ch = [0u8; 8];
        r.read_exact(&mut ch)?;
        let size = be32(&ch[4..8]);
        let body = pos + 8;
        match &ch[0..4] {
            b"COMM" => {
                if size < 18 {
                    return Err(Error::Malformed("COMM chunk too short"));
                }
                let mut c = [0u8; 18];
                r.read_exact(&mut c)?;
                comm = Some(Comm {
                    pos,
                    channels: u16::from_be_bytes([c[0], c[1]]),
                    frames: be32(&c[2..6]),
                    bits: u16::from_be_bytes([c[6], c[7]]),
                    rate: ext80::decode(c[8..18].try_into().unwrap()),
                });
            }
            b"SSND" => {
                let mut s = [0u8; 8];
                r.read_exact(&mut s)?;
                ssnd = Some(Ssnd {
                    pos,
                    size,
                    offset: be32(&s[0..4]),
                });
                if size < 8 || body + size > file_len {
                    // Serato leaves SSND size at 0 until a clean close: audio runs to EOF.
                    extent_inferred = true;
                    break;
                }
            }
            b"ID3 " | b"id3 " if size <= MAX_ID3_BYTES => {
                let mut buf = vec![0u8; size as usize];
                if r.read_exact(&mut buf).is_ok() {
                    tags = id3::parse(&buf);
                }
            }
            _ => {}
        }
        pos = body + size + (size & 1);
    }

    let comm = comm.ok_or(Error::NoComm)?;
    let ssnd = ssnd.ok_or(Error::NoSsnd)?;
    if comm.channels == 0 || !matches!(comm.bits, 8 | 16 | 24 | 32) {
        return Err(Error::Malformed("unsupported channel count or bit depth"));
    }
    let format = Format {
        channels: comm.channels,
        bits: comm.bits,
        sample_rate: comm.rate,
    };
    let align = format.block_align();

    let audio_start = ssnd.pos + 16 + ssnd.offset;
    let data_bytes = if extent_inferred {
        file_len.saturating_sub(audio_start)
    } else {
        ssnd.size.saturating_sub(8 + ssnd.offset)
    };
    let frames = data_bytes / align;
    let audio_bytes = frames * align;

    let mut defects = Vec::new();
    let fixed;
    if extent_inferred {
        let end = audio_start + audio_bytes;
        let pad = audio_bytes & 1;
        fixed = Fixed {
            form_size: end + pad - 8,
            ssnd_size: end - (ssnd.pos + 8),
            frames,
            file_len: end + pad,
        };
        if ssnd.size != fixed.ssnd_size {
            defects.push(Defect::SsndSize {
                found: ssnd.size,
                expected: fixed.ssnd_size,
            });
        }
        if data_bytes != audio_bytes {
            defects.push(Defect::PartialFrame {
                bytes: data_bytes - audio_bytes,
            });
        }
    } else {
        fixed = Fixed {
            form_size: file_len - 8,
            ssnd_size: ssnd.size,
            frames,
            file_len,
        };
    }
    // Serato appends the ID3 chunk after finalising FORM, so a FORM size that stops short of the
    // file end is normal as long as it still covers the audio. When the extent was inferred
    // there is nothing after the audio, so the size must match exactly.
    let form_end = form_size + 8;
    let form_ok = if extent_inferred {
        form_size == fixed.form_size
    } else {
        form_end >= ssnd.pos + 8 + ssnd.size && form_end <= file_len
    };
    if !form_ok {
        defects.push(Defect::FormSize {
            found: form_size,
            expected: fixed.form_size,
        });
    }
    if comm.frames != frames {
        defects.push(Defect::FrameCount {
            found: comm.frames,
            expected: frames,
        });
    }

    Ok(Diagnosis {
        format,
        file_len,
        comm_pos: comm.pos,
        ssnd_pos: ssnd.pos,
        audio_start,
        audio_bytes,
        frames,
        extent_inferred,
        health: if defects.is_empty() {
            Health::Healthy
        } else {
            Health::Broken(defects)
        },
        tags,
        fixed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, Spec};
    use std::io::Cursor;

    fn diag(bytes: &[u8]) -> Result<Diagnosis> {
        diagnose(Cursor::new(bytes), bytes.len() as u64)
    }

    #[test]
    fn healthy_file_is_healthy() {
        let spec = Spec::cd(1000);
        let bytes = fixtures::healthy(spec, 1);
        let d = diag(&bytes).unwrap();
        assert_eq!(d.health, Health::Healthy);
        assert_eq!(d.format, spec.format());
        assert_eq!(d.frames, 1000);
        assert_eq!(d.audio_start, 54);
        assert_eq!(d.audio_bytes, 4000);
        assert!(!d.extent_inferred);
        let tags = d.tags.unwrap();
        assert_eq!(tags.title.as_deref(), Some("Serato Recording"));
        assert_eq!(tags.recorded.as_deref(), Some("2025-10-04 10:31:51"));
    }

    #[test]
    fn healthy_audio_stops_before_the_id3_trailer() {
        let bytes = fixtures::healthy(Spec::cd(10), 1);
        let d = diag(&bytes).unwrap();
        assert!(d.audio_start + d.audio_bytes < bytes.len() as u64);
    }

    #[test]
    fn broken_file_reports_all_zeroed_fields() {
        let bytes = fixtures::broken(Spec::cd(1000), 1, 0);
        let d = diag(&bytes).unwrap();
        assert!(d.extent_inferred);
        assert_eq!(d.frames, 1000);
        assert_eq!(d.audio_bytes, 4000);
        let Health::Broken(defects) = &d.health else {
            panic!("expected broken")
        };
        assert!(defects.contains(&Defect::FormSize {
            found: 0,
            expected: 4046
        }));
        assert!(defects.contains(&Defect::FrameCount {
            found: 0,
            expected: 1000
        }));
        assert!(defects.contains(&Defect::SsndSize {
            found: 0,
            expected: 4008
        }));
        assert_eq!(
            d.fixed,
            Fixed {
                form_size: 4046,
                ssnd_size: 4008,
                frames: 1000,
                file_len: 4054
            }
        );
        assert!(d.tags.is_none());
    }

    #[test]
    fn partial_trailing_frame_is_reported_and_excluded() {
        let bytes = fixtures::broken(Spec::cd(100), 1, 3);
        let d = diag(&bytes).unwrap();
        assert_eq!(d.frames, 100);
        assert_eq!(d.audio_bytes, 400);
        let Health::Broken(defects) = &d.health else {
            panic!()
        };
        assert!(defects.contains(&Defect::PartialFrame { bytes: 3 }));
        assert_eq!(d.fixed.file_len, 454);
    }

    #[test]
    fn reads_format_from_comm_rather_than_assuming() {
        let spec = Spec {
            rate: 48000,
            channels: 1,
            bits: 24,
            frames: 500,
        };
        let d = diag(&fixtures::broken(spec, 2, 0)).unwrap();
        assert_eq!(d.format, spec.format());
        assert_eq!(d.frames, 500);
        assert_eq!(d.audio_bytes, 1500);
    }

    #[test]
    fn odd_length_audio_gets_a_pad_byte_in_the_fixed_length() {
        let spec = Spec {
            rate: 8000,
            channels: 1,
            bits: 8,
            frames: 5,
        };
        let d = diag(&fixtures::broken(spec, 0, 0)).unwrap();
        assert_eq!(d.fixed.file_len, 54 + 5 + 1);
        assert_eq!(d.fixed.form_size, 54 + 5 + 1 - 8);
    }

    #[test]
    fn wrong_frame_count_alone_is_a_defect_without_inferring_the_extent() {
        let mut bytes = fixtures::healthy(Spec::cd(100), 1);
        bytes[22..26].copy_from_slice(&7u32.to_be_bytes());
        let d = diag(&bytes).unwrap();
        assert!(!d.extent_inferred);
        assert_eq!(d.frames, 100);
        assert_eq!(
            d.health,
            Health::Broken(vec![Defect::FrameCount {
                found: 7,
                expected: 100
            }])
        );
        assert_eq!(d.fixed.file_len, bytes.len() as u64);
    }

    #[test]
    fn form_size_that_stops_before_the_id3_trailer_is_still_healthy() {
        // Real Serato files do this: the tag is appended after FORM was finalised.
        let mut bytes = fixtures::healthy(Spec::cd(100), 1);
        let trailer = fixtures::id3_chunk("Recorded 2025-10-04 10:31:51").len();
        let form = (bytes.len() - 8 - trailer) as u32;
        bytes[4..8].copy_from_slice(&form.to_be_bytes());
        let d = diag(&bytes).unwrap();
        assert_eq!(d.health, Health::Healthy);
        assert!(d.tags.is_some());
    }

    #[test]
    fn form_size_that_cuts_into_the_audio_or_overshoots_the_file_is_broken() {
        let good = fixtures::healthy(Spec::cd(100), 1);
        for bad in [100u32, good.len() as u32] {
            let mut bytes = good.clone();
            bytes[4..8].copy_from_slice(&bad.to_be_bytes());
            let d = diag(&bytes).unwrap();
            assert!(
                matches!(&d.health, Health::Broken(v) if v.iter().any(|x| matches!(x, Defect::FormSize { .. }))),
                "form={bad}"
            );
        }
    }

    #[test]
    fn rejects_non_aiff() {
        assert!(matches!(diag(b"RIFF....WAVEfmt "), Err(Error::NotAiff)));
        assert!(matches!(diag(b""), Err(Error::NotAiff)));
    }

    #[test]
    fn aifc_is_unsupported() {
        let mut bytes = fixtures::healthy(Spec::cd(10), 1);
        bytes[8..12].copy_from_slice(b"AIFC");
        assert!(matches!(diag(&bytes), Err(Error::Unsupported(_))));
    }

    #[test]
    fn header_without_audio_chunk_is_an_error() {
        let bytes = fixtures::healthy(Spec::cd(10), 1);
        assert!(matches!(diag(&bytes[..38]), Err(Error::NoSsnd)));
    }
}
