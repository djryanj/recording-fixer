//! Non-destructive edit lists: ordered, trimmed clips of PCM audio that are only
//! rendered when exported. Nothing here ever modifies a source file.

pub mod fade;

use aiff_core::{diagnose_path, header, Format, Progress};
pub use fade::{Fade, FadeReader};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub use aiff_core::Progress as ProgressFn;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("nothing to combine")]
    Empty,
    #[error("clip {index} ({path}): {source}")]
    Source {
        index: usize,
        path: String,
        source: aiff_core::Error,
    },
    #[error("clip {index} is {found}, but the first clip is {expected}; resampling is not supported yet")]
    FormatMismatch {
        index: usize,
        found: String,
        expected: String,
    },
    #[error("clip {index} has an invalid range")]
    BadRange { index: usize },
    #[error(
        "the combined audio is larger than AIFF's 4 GB limit; export to another format instead"
    )]
    TooLarge,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

/// A slice of one source file, in frames. `end_frame: None` means "to the end".
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub path: PathBuf,
    pub start_frame: u64,
    pub end_frame: Option<u64>,
}

impl Clip {
    pub fn whole(path: impl Into<PathBuf>) -> Self {
        Clip {
            path: path.into(),
            start_frame: 0,
            end_frame: None,
        }
    }
}

#[derive(Debug, Clone)]
struct Resolved {
    path: PathBuf,
    /// File offset of the first byte of this clip's audio.
    offset: u64,
    bytes: u64,
}

/// A validated edit list: every clip exists, shares one format, and has a sane range.
#[derive(Debug, Clone)]
pub struct Plan {
    pub format: Format,
    pub total_frames: u64,
    clips: Vec<Resolved>,
}

fn describe(f: &Format) -> String {
    format!("{} Hz / {}-bit / {} ch", f.sample_rate, f.bits, f.channels)
}

/// AIFF sizes are 32-bit: the header plus audio (and pad) must fit.
pub fn fits_in_aiff(audio_bytes: u64) -> bool {
    header::HEADER_LEN - 8 + audio_bytes + (audio_bytes & 1) <= u64::from(u32::MAX)
        && audio_bytes + 8 <= u64::from(u32::MAX)
}

pub fn plan(clips: &[Clip]) -> Result<Plan> {
    let mut format: Option<Format> = None;
    let mut resolved = Vec::with_capacity(clips.len());
    let mut total_frames = 0u64;
    for (index, clip) in clips.iter().enumerate() {
        let d = diagnose_path(&clip.path).map_err(|source| Error::Source {
            index,
            path: clip.path.display().to_string(),
            source,
        })?;
        match &format {
            None => format = Some(d.format.clone()),
            Some(f) if *f != d.format => {
                return Err(Error::FormatMismatch {
                    index,
                    found: describe(&d.format),
                    expected: describe(f),
                })
            }
            Some(_) => {}
        }
        let end = clip.end_frame.unwrap_or(d.frames);
        if clip.start_frame >= end || end > d.frames {
            return Err(Error::BadRange { index });
        }
        let align = d.format.block_align();
        resolved.push(Resolved {
            path: clip.path.clone(),
            offset: d.audio_start + clip.start_frame * align,
            bytes: (end - clip.start_frame) * align,
        });
        total_frames += end - clip.start_frame;
    }
    let format = format.ok_or(Error::Empty)?;
    Ok(Plan {
        format,
        total_frames,
        clips: resolved,
    })
}

impl Plan {
    pub fn total_bytes(&self) -> u64 {
        self.clips.iter().map(|c| c.bytes).sum()
    }

    pub fn duration_secs(&self) -> f64 {
        self.total_frames as f64 / self.format.sample_rate
    }

    /// Streams the raw big-endian PCM of all clips back to back.
    pub fn reader(&self) -> PlanReader {
        PlanReader {
            clips: self.clips.clone(),
            next: 0,
            current: None,
            remaining: 0,
        }
    }

    /// Like [`reader`](Self::reader) but starting `frame` frames into the combined audio.
    /// A `frame` at or past the end yields an empty reader.
    pub fn reader_from(&self, frame: u64) -> PlanReader {
        let align = self.format.block_align();
        let mut skip = frame.saturating_mul(align);
        let mut clips = Vec::with_capacity(self.clips.len());
        for c in &self.clips {
            if skip >= c.bytes {
                skip -= c.bytes;
                continue;
            }
            let mut c = c.clone();
            c.offset += skip;
            c.bytes -= skip;
            skip = 0;
            clips.push(c);
        }
        PlanReader {
            clips,
            next: 0,
            current: None,
            remaining: 0,
        }
    }

    /// Render the plan to a new AIFF file. Refuses to overwrite `dst`.
    pub fn write_aiff(&self, dst: &Path, progress: Progress) -> Result<()> {
        self.write_aiff_faded(dst, Fade::default(), progress)
    }

    /// The combined audio with a fade in and/or out applied to its very start and end.
    pub fn reader_faded(&self, fade: Fade) -> FadeReader<PlanReader> {
        FadeReader::new(
            self.reader(),
            fade,
            self.format.bits,
            self.format.channels,
            self.total_frames,
        )
    }

    /// Like [`write_aiff`](Self::write_aiff), with fades.
    pub fn write_aiff_faded(&self, dst: &Path, fade: Fade, progress: Progress) -> Result<()> {
        self.write_aiff_from(dst, &mut self.reader_faded(fade), progress)
    }

    /// Like [`write_aiff`](Self::write_aiff), but taking the audio from `source`: the plan's own
    /// format and length, with any processing (fades, loudness) already applied by the caller.
    pub fn write_aiff_from(
        &self,
        dst: &Path,
        source: &mut dyn Read,
        progress: Progress,
    ) -> Result<()> {
        let bytes = self.total_bytes();
        if !fits_in_aiff(bytes) || self.total_frames > u64::from(u32::MAX) {
            return Err(Error::TooLarge);
        }
        let mut out = OpenOptions::new().write(true).create_new(true).open(dst)?;
        let result = (|| {
            out.write_all(&header::build(
                &self.format,
                self.total_frames as u32,
                bytes,
            ))?;
            pump(source, &mut out, bytes, progress)?;
            if bytes & 1 == 1 {
                out.write_all(&[0])?;
            }
            out.flush()?;
            Ok(())
        })();
        if result.is_err() {
            drop(out);
            let _ = std::fs::remove_file(dst);
        }
        result
    }
}

pub struct PlanReader {
    clips: Vec<Resolved>,
    next: usize,
    current: Option<File>,
    remaining: u64,
}

impl Read for PlanReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            // Clips are validated to be non-empty, so opening the next one always yields bytes.
            let Some(clip) = self.clips.get(self.next) else {
                return Ok(0);
            };
            let mut f = File::open(&clip.path)?;
            f.seek(SeekFrom::Start(clip.offset))?;
            self.current = Some(f);
            self.remaining = clip.bytes;
            self.next += 1;
        }
        let want = buf
            .len()
            .min(self.remaining.min(usize::MAX as u64) as usize);
        let n = self.current.as_mut().unwrap().read(&mut buf[..want])?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "source file ended early",
            ));
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}

const PUMP_BUF: usize = 1 << 20;

/// Copy exactly `total` bytes from `r` to `w` through a fixed buffer, reporting progress.
pub fn pump<R: Read + ?Sized, W: Write + ?Sized>(
    r: &mut R,
    w: &mut W,
    total: u64,
    progress: Progress,
) -> Result<()> {
    let mut buf = vec![0u8; PUMP_BUF];
    let mut done = 0u64;
    while done < total {
        let want = (total - done).min(PUMP_BUF as u64) as usize;
        r.read_exact(&mut buf[..want])?;
        w.write_all(&buf[..want])?;
        done += want as u64;
        if !progress(done, total) {
            return Err(Error::Cancelled);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiff_core::fixtures::{self, Spec};
    use aiff_core::{diagnose_path, Health};
    use std::fs;

    fn put(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.path().join(name);
        fs::write(&p, bytes).unwrap();
        p
    }

    fn read_all(plan: &Plan) -> Vec<u8> {
        let mut v = Vec::new();
        plan.reader().read_to_end(&mut v).unwrap();
        v
    }

    fn go() -> impl FnMut(u64, u64) -> bool {
        |_, _| true
    }

    #[test]
    fn empty_list_is_an_error() {
        assert!(matches!(plan(&[]), Err(Error::Empty)));
    }

    #[test]
    fn two_whole_clips_concatenate_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (Spec::cd(300), Spec::cd(200));
        let pa = put(&dir, "a.aif", &fixtures::healthy(a, 1));
        let pb = put(&dir, "b.aif", &fixtures::healthy(b, 2));
        let p = plan(&[Clip::whole(pa), Clip::whole(pb)]).unwrap();
        assert_eq!(p.total_frames, 500);
        let mut want = fixtures::pcm(a, 1);
        want.extend(fixtures::pcm(b, 2));
        assert_eq!(read_all(&p), want);
    }

    #[test]
    fn trimming_selects_the_requested_frames() {
        let dir = tempfile::tempdir().unwrap();
        let spec = Spec::cd(1000);
        let pa = put(&dir, "a.aif", &fixtures::healthy(spec, 1));
        let p = plan(&[Clip {
            path: pa,
            start_frame: 100,
            end_frame: Some(400),
        }])
        .unwrap();
        assert_eq!(p.total_frames, 300);
        assert_eq!(read_all(&p), fixtures::pcm(spec, 1)[400..1600]);
    }

    #[test]
    fn a_broken_source_can_be_used_without_repairing_it_first() {
        let dir = tempfile::tempdir().unwrap();
        let spec = Spec::cd(500);
        let pa = put(&dir, "broken.aif", &fixtures::broken(spec, 5, 3));
        let p = plan(&[Clip::whole(pa)]).unwrap();
        assert_eq!(p.total_frames, 500);
        assert_eq!(read_all(&p), fixtures::pcm(spec, 5));
    }

    #[test]
    fn the_id3_trailer_is_never_treated_as_audio() {
        let dir = tempfile::tempdir().unwrap();
        let spec = Spec::cd(50);
        let pa = put(&dir, "a.aif", &fixtures::healthy(spec, 1));
        assert_eq!(
            read_all(&plan(&[Clip::whole(pa)]).unwrap()),
            fixtures::pcm(spec, 1)
        );
    }

    #[test]
    fn mismatched_sample_rates_are_rejected_with_a_clear_message() {
        let dir = tempfile::tempdir().unwrap();
        let pa = put(&dir, "a.aif", &fixtures::healthy(Spec::cd(10), 1));
        let pb = put(
            &dir,
            "b.aif",
            &fixtures::healthy(
                Spec {
                    rate: 48000,
                    ..Spec::cd(10)
                },
                1,
            ),
        );
        let err = plan(&[Clip::whole(pa), Clip::whole(pb)]).unwrap_err();
        assert!(matches!(err, Error::FormatMismatch { index: 1, .. }));
        let msg = err.to_string();
        assert!(msg.contains("48000") && msg.contains("44100"), "{msg}");
    }

    #[test]
    fn invalid_ranges_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let pa = put(&dir, "a.aif", &fixtures::healthy(Spec::cd(100), 1));
        for (s, e) in [(50, Some(50)), (60, Some(10)), (0, Some(101)), (100, None)] {
            let c = Clip {
                path: pa.clone(),
                start_frame: s,
                end_frame: e,
            };
            assert!(
                matches!(plan(&[c]), Err(Error::BadRange { index: 0 })),
                "{s}..{e:?}"
            );
        }
    }

    #[test]
    fn a_missing_file_names_the_clip() {
        let err = plan(&[Clip::whole("does-not-exist.aif")]).unwrap_err();
        assert!(matches!(err, Error::Source { index: 0, .. }));
    }

    #[test]
    fn write_aiff_produces_a_healthy_combined_file() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (Spec::cd(3000), Spec::cd(2001));
        let pa = put(&dir, "a.aif", &fixtures::broken(a, 1, 0));
        let pb = put(&dir, "b.aif", &fixtures::healthy(b, 2));
        let out = dir.path().join("joined.aif");
        plan(&[Clip::whole(pa), Clip::whole(pb)])
            .unwrap()
            .write_aiff(&out, &mut go())
            .unwrap();
        let d = diagnose_path(&out).unwrap();
        assert_eq!(d.health, Health::Healthy);
        assert_eq!(d.frames, 5001);
        let all = fs::read(&out).unwrap();
        let mut want = fixtures::pcm(a, 1);
        want.extend(fixtures::pcm(b, 2));
        assert_eq!(&all[54..], &want[..]);
    }

    #[test]
    fn write_aiff_never_overwrites_and_cleans_up_on_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let pa = put(&dir, "a.aif", &fixtures::healthy(Spec::cd(400_000), 1));
        let p = plan(&[Clip::whole(pa)]).unwrap();
        let existing = put(&dir, "x.aif", b"keep");
        assert!(p.write_aiff(&existing, &mut go()).is_err());
        assert_eq!(fs::read(&existing).unwrap(), b"keep");

        let out = dir.path().join("cancelled.aif");
        assert!(matches!(
            p.write_aiff(&out, &mut |_, _| false),
            Err(Error::Cancelled)
        ));
        assert!(!out.exists());
    }

    #[test]
    fn reader_from_starts_mid_clip() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (Spec::cd(300), Spec::cd(200));
        let pa = put(&dir, "a.aif", &fixtures::healthy(a, 1));
        let pb = put(&dir, "b.aif", &fixtures::healthy(b, 2));
        let p = plan(&[Clip::whole(pa), Clip::whole(pb)]).unwrap();
        let mut want = fixtures::pcm(a, 1);
        want.extend(fixtures::pcm(b, 2));
        for frame in [0u64, 1, 150, 299, 300, 301, 450, 499] {
            let mut got = Vec::new();
            p.reader_from(frame).read_to_end(&mut got).unwrap();
            assert_eq!(got, want[frame as usize * 4..], "from frame {frame}");
        }
    }

    #[test]
    fn reader_from_the_end_or_beyond_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let pa = put(&dir, "a.aif", &fixtures::healthy(Spec::cd(100), 1));
        let p = plan(&[Clip::whole(pa)]).unwrap();
        for frame in [100u64, 101, u64::MAX] {
            let mut got = Vec::new();
            p.reader_from(frame).read_to_end(&mut got).unwrap();
            assert!(got.is_empty(), "from frame {frame}");
        }
    }

    #[test]
    fn reader_from_respects_trims() {
        let dir = tempfile::tempdir().unwrap();
        let spec = Spec::cd(1000);
        let pa = put(&dir, "a.aif", &fixtures::healthy(spec, 3));
        let p = plan(&[Clip {
            path: pa,
            start_frame: 100,
            end_frame: Some(400),
        }])
        .unwrap();
        let mut got = Vec::new();
        p.reader_from(50).read_to_end(&mut got).unwrap();
        assert_eq!(got, fixtures::pcm(spec, 3)[150 * 4..400 * 4]);
    }

    #[test]
    fn a_faded_aiff_is_silent_at_both_ends_and_identical_in_the_middle() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (Spec::cd(30_000), Spec::cd(20_000));
        let pa = put(&dir, "a.aif", &fixtures::healthy(a, 1));
        let pb = put(&dir, "b.aif", &fixtures::healthy(b, 2));
        let p = plan(&[Clip::whole(pa), Clip::whole(pb)]).unwrap();
        let out = dir.path().join("faded.aif");
        let fade = Fade {
            in_frames: 4000,
            out_frames: 6000,
        };
        p.write_aiff_faded(&out, fade, &mut go()).unwrap();

        let d = diagnose_path(&out).unwrap();
        assert_eq!(d.health, Health::Healthy);
        assert_eq!(d.frames, 50_000);
        let all = fs::read(&out).unwrap();
        let audio = &all[54..];
        let mut plain = fixtures::pcm(a, 1);
        plain.extend(fixtures::pcm(b, 2));
        // First and last frame are silent.
        assert!(audio[..4].iter().all(|&x| x == 0));
        assert!(audio[audio.len() - 4..].iter().all(|&x| x == 0));
        // Between the fades nothing is touched, bit for bit.
        let (mid_a, mid_b) = (4000 * 4, (50_000 - 6000) * 4);
        assert_eq!(&audio[mid_a..mid_b], &plain[mid_a..mid_b]);
        // The fades themselves changed the audio.
        assert_ne!(&audio[4 * 100..4 * 3000], &plain[4 * 100..4 * 3000]);
    }

    #[test]
    fn the_unfaded_writer_is_bit_identical_to_a_zero_fade() {
        let dir = tempfile::tempdir().unwrap();
        let pa = put(&dir, "a.aif", &fixtures::healthy(Spec::cd(5000), 1));
        let p = plan(&[Clip::whole(pa)]).unwrap();
        let (x, y) = (dir.path().join("x.aif"), dir.path().join("y.aif"));
        p.write_aiff(&x, &mut go()).unwrap();
        p.write_aiff_faded(&y, Fade::default(), &mut go()).unwrap();
        assert_eq!(fs::read(x).unwrap(), fs::read(y).unwrap());
    }

    #[test]
    fn fades_apply_to_the_whole_combined_result_not_to_each_clip() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (Spec::cd(1000), Spec::cd(1000));
        let pa = put(&dir, "a.aif", &fixtures::healthy(a, 1));
        let pb = put(&dir, "b.aif", &fixtures::healthy(b, 2));
        let p = plan(&[Clip::whole(pa), Clip::whole(pb)]).unwrap();
        let mut got = Vec::new();
        p.reader_faded(Fade {
            in_frames: 100,
            out_frames: 100,
        })
        .read_to_end(&mut got)
        .unwrap();
        let mut plain = fixtures::pcm(a, 1);
        plain.extend(fixtures::pcm(b, 2));
        // The join between the two clips (frame 1000) is untouched: no dip there.
        assert_eq!(&got[990 * 4..1010 * 4], &plain[990 * 4..1010 * 4]);
    }

    #[test]
    fn aiff_size_limit_boundaries() {
        assert!(fits_in_aiff(1_000_000));
        assert!(fits_in_aiff(u64::from(u32::MAX) - 100));
        assert!(!fits_in_aiff(u64::from(u32::MAX)));
        assert!(!fits_in_aiff(5_000_000_000));
    }
}
