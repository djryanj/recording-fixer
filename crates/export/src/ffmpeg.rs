//! Locating ffmpeg, building its command line, and streaming audio into it.

use crate::options::{AacMode, Codec, Metadata, Mp3Mode, APPLE_ENCODER};
use crate::{encoders, Encoders, Error, Result};
use aiff_core::{Format, Progress};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct ExportSpec {
    /// Format of the raw big-endian PCM that will be written to the encoder.
    pub format: Format,
    pub codec: Codec,
    pub output: PathBuf,
    pub metadata: Metadata,
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// Resolution order: the user's explicit choice, a bundled copy, then PATH.
pub fn locate_ffmpeg(user: Option<&Path>, bundled_dirs: &[PathBuf]) -> Option<PathBuf> {
    if let Some(p) = user.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    for dir in bundled_dirs {
        let p = dir.join(exe_name());
        if p.is_file() {
            return Some(p);
        }
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|d| d.join(exe_name()))
        .find(|p| p.is_file())
}

fn command(ffmpeg: &Path) -> Command {
    let mut cmd = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

pub fn probe_encoders(ffmpeg: &Path) -> Result<Encoders> {
    let out = command(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()?;
    Ok(encoders::parse_encoders(&String::from_utf8_lossy(
        &out.stdout,
    )))
}

/// First line of `ffmpeg -version`, e.g. "ffmpeg version 9.0.2-full_build ...".
pub fn version(ffmpeg: &Path) -> Option<String> {
    let out = command(ffmpeg).arg("-version").output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(str::to_string)
}

fn pcm_input_format(bits: u16) -> &'static str {
    match bits {
        8 => "s8",
        16 => "s16be",
        24 => "s24be",
        _ => "s32be",
    }
}

const COMMON: [&str; 7] = [
    "-hide_banner",
    "-loglevel",
    "error",
    "-nostats",
    "-progress",
    "pipe:1",
    "-n",
];

/// Codec-specific output options for an ffmpeg AAC encoder. Level scales differ per encoder.
fn aac_args(encoder: &str, mode: &AacMode) -> Vec<String> {
    let mut a = vec!["-c:a".to_string(), encoder.to_string()];
    let cbr = |a: &mut Vec<String>, kbps: u32| a.extend(["-b:a".to_string(), format!("{kbps}k")]);
    match (encoder, mode) {
        ("aac_at", AacMode::Cbr(k)) => {
            a.extend(["-aac_at_mode".into(), "cbr".into()]);
            cbr(&mut a, *k);
        }
        ("aac_at", AacMode::Vbr(l)) => {
            // aac_at quality runs 0 (best) to 14; level 5 is best.
            let q = [12, 9, 6, 3, 1][usize::from((*l).clamp(1, 5)) - 1];
            a.extend([
                "-aac_at_mode".into(),
                "vbr".into(),
                "-q:a".into(),
                q.to_string(),
            ]);
        }
        ("libfdk_aac", AacMode::Vbr(l)) => a.extend(["-vbr".into(), l.clamp(&1, &5).to_string()]),
        // The native encoder's quality scale is 0.1 to 2 (higher is better).
        ("aac", AacMode::Vbr(l)) => a.extend([
            "-q:a".into(),
            format!("{:.1}", f64::from((*l).clamp(1, 5)) * 0.4),
        ]),
        // aac_mf and anything unknown have no usable VBR: approximate with a CBR rate.
        (_, mode) => cbr(&mut a, mode.fallback_kbps()),
    }
    if let ("libfdk_aac" | "aac", AacMode::Cbr(k)) = (encoder, mode) {
        cbr(&mut a, *k);
    }
    a
}

fn metadata_args(m: &Metadata) -> Vec<String> {
    m.pairs()
        .into_iter()
        .flat_map(|(k, v)| ["-metadata".to_string(), format!("{k}={v}")])
        .collect()
}

fn finish(mut a: Vec<String>, spec: &ExportSpec) -> Vec<OsString> {
    a.extend(metadata_args(&spec.metadata));
    let mut out: Vec<OsString> = a.into_iter().map(OsString::from).collect();
    out.push(spec.output.clone().into_os_string());
    out
}

/// ffmpeg arguments that read raw PCM on stdin and encode it.
pub fn build_args(spec: &ExportSpec) -> Vec<OsString> {
    let mut a: Vec<String> = COMMON.map(String::from).into();
    a.extend([
        "-f".into(),
        pcm_input_format(spec.format.bits).into(),
        "-ar".into(),
        format!("{}", spec.format.sample_rate.round() as u64),
        "-ac".into(),
        spec.format.channels.to_string(),
        "-i".into(),
        "pipe:0".into(),
        "-vn".into(),
    ]);
    match &spec.codec {
        Codec::Mp3(o) => {
            a.extend(["-c:a".into(), "libmp3lame".into()]);
            match o.mode {
                Mp3Mode::Cbr(k) => a.extend(["-b:a".into(), format!("{k}k")]),
                Mp3Mode::Vbr(v) => a.extend(["-q:a".into(), v.min(9).to_string()]),
                Mp3Mode::Abr(k) => {
                    a.extend(["-abr".into(), "1".into(), "-b:a".into(), format!("{k}k")])
                }
            }
            a.extend([
                "-id3v2_version".into(),
                o.id3_version.clamp(3, 4).to_string(),
            ]);
            if o.id3v1 {
                a.extend(["-write_id3v1".into(), "1".into()]);
            }
        }
        Codec::Aac { encoder, mode } => {
            a.extend(aac_args(encoder, mode));
            a.extend(["-movflags".into(), "+faststart".into()]);
        }
    }
    finish(a, spec)
}

/// ffmpeg arguments that wrap ADTS AAC from stdin into an .m4a without re-encoding.
///
/// `delay_frames` is the encoder delay: the encoder has prepended that much silence, so the
/// stream is timestamped that far *before* zero. ffmpeg then records an edit list that makes
/// players skip it, and the audio starts exactly where it should. (Padding more silence at the
/// start would not help; it would double the offset.)
pub fn build_mux_args(spec: &ExportSpec, delay_frames: u32) -> Vec<OsString> {
    let mut a: Vec<String> = COMMON.map(String::from).into();
    if delay_frames > 0 {
        let secs = f64::from(delay_frames) / spec.format.sample_rate;
        a.extend(["-itsoffset".to_string(), format!("-{secs:.7}")]);
    }
    a.extend(
        [
            "-f",
            "aac",
            "-i",
            "pipe:0",
            "-vn",
            "-c:a",
            "copy",
            "-movflags",
            "+faststart",
        ]
        .map(String::from),
    );
    finish(a, spec)
}

/// Feeds audio into ffmpeg's stdin. Returning `Err(Cancelled)` kills ffmpeg.
type Feed<'a> = &'a mut dyn FnMut(&mut dyn Write) -> Result<()>;

fn tail(s: &str, max: usize) -> String {
    let start = s.len().saturating_sub(max);
    let start = (start..=s.len())
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(s.len());
    s[start..].trim().to_string()
}

/// Spawn ffmpeg, let `feed` write to its stdin, and clean up the output on any failure.
fn with_ffmpeg(ffmpeg: &Path, output: &Path, args: Vec<OsString>, feed: Feed) -> Result<()> {
    if output.exists() {
        return Err(Error::OutputExists(output.display().to_string()));
    }
    let mut child = command(ffmpeg)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    // Drain both pipes so ffmpeg can never block on a full buffer.
    let drain_out = std::thread::spawn(move || {
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
    });
    let drain_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        tail(&String::from_utf8_lossy(&buf), 2000)
    });

    let fed = feed(&mut stdin);
    drop(stdin);
    if matches!(fed, Err(Error::Cancelled)) {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let _ = drain_out.join();
    let log = drain_err.join().unwrap_or_default();

    let result = match fed {
        Err(Error::Cancelled) => Err(Error::Cancelled),
        // A broken pipe means ffmpeg exited early; its log explains why.
        Err(Error::Io(_)) if !log.is_empty() => Err(Error::Ffmpeg {
            status: status.to_string(),
            log,
        }),
        Err(e) => Err(e),
        Ok(()) if !status.success() => Err(Error::Ffmpeg {
            status: status.to_string(),
            log,
        }),
        Ok(()) => Ok(()),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(output);
    }
    result
}

/// Encode `total_bytes` of PCM from `source` to `spec.output`. On failure or cancel the partial
/// output is removed.
pub fn run(
    ffmpeg: &Path,
    spec: &ExportSpec,
    source: &mut dyn Read,
    total_bytes: u64,
    progress: Progress,
) -> Result<()> {
    if let Codec::Aac { encoder, mode } = &spec.codec {
        if encoder == APPLE_ENCODER {
            return run_apple(ffmpeg, spec, mode, source, total_bytes, progress);
        }
    }
    with_ffmpeg(ffmpeg, &spec.output, build_args(spec), &mut |w| {
        edit_core::pump(source, w, total_bytes, progress).map_err(Error::from)
    })
}

#[cfg(windows)]
fn run_apple(
    ffmpeg: &Path,
    spec: &ExportSpec,
    mode: &AacMode,
    source: &mut dyn Read,
    total_bytes: u64,
    progress: Progress,
) -> Result<()> {
    let dll = crate::apple::usable_windows_dll().ok_or_else(|| {
        Error::Apple("Apple Application Support (CoreAudioToolbox.dll) was not found".into())
    })?;
    let delay = crate::apple_aac::priming_frames(&dll, &spec.format, mode);
    with_ffmpeg(
        ffmpeg,
        &spec.output,
        build_mux_args(spec, delay),
        &mut |w| {
            crate::apple_aac::encode(&dll, &spec.format, mode, source, total_bytes, w, progress)
        },
    )
}

#[cfg(not(windows))]
fn run_apple(
    _: &Path,
    _: &ExportSpec,
    _: &AacMode,
    _: &mut dyn Read,
    _: u64,
    _: Progress,
) -> Result<()> {
    Err(Error::Apple(
        "direct Apple AAC encoding is only available on Windows; use ffmpeg's aac_at".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::Mp3Options;
    use aiff_core::fixtures::{self, Spec};
    use edit_core::Clip;
    use std::fs;

    fn spec(codec: Codec) -> ExportSpec {
        ExportSpec {
            format: Spec::cd(0).format(),
            codec,
            output: "out.mp3".into(),
            metadata: Metadata::default(),
        }
    }

    fn mp3(mode: Mp3Mode) -> Codec {
        Codec::Mp3(Mp3Options::new(mode))
    }

    fn aac(encoder: &str, mode: AacMode) -> Codec {
        Codec::Aac {
            encoder: encoder.into(),
            mode,
        }
    }

    fn strings(spec: &ExportSpec) -> Vec<String> {
        build_args(spec)
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    fn has_pair(args: &[String], k: &str, v: &str) -> bool {
        args.windows(2).any(|w| w[0] == k && w[1] == v)
    }

    #[test]
    fn mp3_cbr_arguments() {
        let a = strings(&spec(mp3(Mp3Mode::Cbr(320))));
        assert!(has_pair(&a, "-c:a", "libmp3lame") && has_pair(&a, "-b:a", "320k"));
        assert!(
            has_pair(&a, "-f", "s16be") && has_pair(&a, "-ar", "44100") && has_pair(&a, "-ac", "2")
        );
        assert!(has_pair(&a, "-i", "pipe:0"));
        assert_eq!(a.last().unwrap(), "out.mp3");
    }

    #[test]
    fn mp3_vbr_is_clamped_to_lame_range() {
        assert!(has_pair(
            &strings(&spec(mp3(Mp3Mode::Vbr(42)))),
            "-q:a",
            "9"
        ));
        assert!(has_pair(&strings(&spec(mp3(Mp3Mode::Vbr(0)))), "-q:a", "0"));
    }

    #[test]
    fn mp3_abr_turns_on_average_bitrate_mode() {
        let a = strings(&spec(mp3(Mp3Mode::Abr(192))));
        assert!(has_pair(&a, "-abr", "1") && has_pair(&a, "-b:a", "192k"));
    }

    #[test]
    fn id3_version_and_v1_are_options() {
        let mut o = Mp3Options::new(Mp3Mode::Cbr(192));
        assert!(has_pair(
            &strings(&spec(Codec::Mp3(o.clone()))),
            "-id3v2_version",
            "3"
        ));
        assert!(!strings(&spec(Codec::Mp3(o.clone()))).contains(&"-write_id3v1".to_string()));
        o.id3_version = 4;
        o.id3v1 = true;
        let a = strings(&spec(Codec::Mp3(o)));
        assert!(has_pair(&a, "-id3v2_version", "4") && has_pair(&a, "-write_id3v1", "1"));
    }

    #[test]
    fn aac_cbr_uses_the_chosen_encoder() {
        for enc in ["aac", "libfdk_aac", "aac_mf", "aac_at"] {
            let a = strings(&spec(aac(enc, AacMode::Cbr(256))));
            assert!(
                has_pair(&a, "-c:a", enc) && has_pair(&a, "-b:a", "256k"),
                "{enc}"
            );
            assert!(has_pair(&a, "-movflags", "+faststart"));
        }
        assert!(has_pair(
            &strings(&spec(aac("aac_at", AacMode::Cbr(256)))),
            "-aac_at_mode",
            "cbr"
        ));
    }

    #[test]
    fn aac_vbr_maps_to_each_encoders_own_scale() {
        let a = strings(&spec(aac("libfdk_aac", AacMode::Vbr(4))));
        assert!(has_pair(&a, "-vbr", "4") && !a.contains(&"-b:a".to_string()));
        let a = strings(&spec(aac("aac", AacMode::Vbr(5))));
        assert!(has_pair(&a, "-q:a", "2.0"));
        let a = strings(&spec(aac("aac_at", AacMode::Vbr(5))));
        assert!(has_pair(&a, "-aac_at_mode", "vbr") && has_pair(&a, "-q:a", "1"));
        let a = strings(&spec(aac("aac_at", AacMode::Vbr(1))));
        assert!(has_pair(&a, "-q:a", "12"));
    }

    #[test]
    fn encoders_without_vbr_fall_back_to_a_cbr_rate() {
        let a = strings(&spec(aac("aac_mf", AacMode::Vbr(3))));
        assert!(has_pair(&a, "-b:a", "192k"));
    }

    #[test]
    fn input_format_follows_bit_depth_and_rate() {
        let mut s = spec(mp3(Mp3Mode::Cbr(192)));
        s.format = Spec {
            rate: 48000,
            channels: 1,
            bits: 24,
            frames: 0,
        }
        .format();
        let a = strings(&s);
        assert!(
            has_pair(&a, "-f", "s24be") && has_pair(&a, "-ar", "48000") && has_pair(&a, "-ac", "1")
        );
    }

    #[test]
    fn never_overwrites_and_metadata_goes_in_tags() {
        let mut s = spec(mp3(Mp3Mode::Cbr(192)));
        s.metadata = Metadata {
            title: "My Set".into(),
            artist: "DJ".into(),
            genre: "House".into(),
            ..Default::default()
        };
        let a = strings(&s);
        assert!(a.contains(&"-n".to_string()));
        assert!(has_pair(&a, "-metadata", "title=My Set"));
        assert!(has_pair(&a, "-metadata", "artist=DJ") && has_pair(&a, "-metadata", "genre=House"));
        assert!(!a.iter().any(|x| x.starts_with("album=")));
    }

    #[test]
    fn mux_args_copy_the_stream_and_keep_tags() {
        let mut s = spec(aac(APPLE_ENCODER, AacMode::Cbr(256)));
        s.output = "o.m4a".into();
        s.metadata.title = "T".into();
        let a: Vec<String> = build_mux_args(&s, 2112)
            .iter()
            .map(|x| x.to_string_lossy().into_owned())
            .collect();
        assert!(
            has_pair(&a, "-f", "aac")
                && has_pair(&a, "-c:a", "copy")
                && has_pair(&a, "-metadata", "title=T")
        );
        assert!(!a.contains(&"-b:a".to_string()));
    }

    #[test]
    fn mux_shifts_the_stream_back_by_the_encoder_delay_before_the_input() {
        let s = spec(aac(APPLE_ENCODER, AacMode::Cbr(256)));
        let a: Vec<String> = build_mux_args(&s, 2112)
            .iter()
            .map(|x| x.to_string_lossy().into_owned())
            .collect();
        // 2112 / 44100 = 0.0478911...
        assert!(has_pair(&a, "-itsoffset", "-0.0478912"), "{a:?}");
        let (off, input) = (
            a.iter().position(|x| x == "-itsoffset").unwrap(),
            a.iter().position(|x| x == "-i").unwrap(),
        );
        assert!(
            off < input,
            "-itsoffset only applies to the input that follows it"
        );
        let none: Vec<String> = build_mux_args(&s, 0)
            .iter()
            .map(|x| x.to_string_lossy().into_owned())
            .collect();
        assert!(!none.contains(&"-itsoffset".to_string()));
    }

    /// Encode a steady loud tone with a 0.6 s fade in and out, decode it, and return the RMS level
    /// (0..1) of the first 100 ms, the middle 100 ms and the last 100 ms.
    fn faded_levels(codec: Codec) -> Option<(f64, f64, f64)> {
        let (ffmpeg, _) = tools()?;
        let dir = tempfile::tempdir().unwrap();
        let rate = 44100u32;
        let frames = rate * 3;
        let mut audio = Vec::new();
        for i in 0..frames {
            let v = ((f64::from(i) * 440.0 * std::f64::consts::TAU / f64::from(rate)).sin()
                * 20000.0) as i16;
            audio.extend_from_slice(&v.to_be_bytes());
            audio.extend_from_slice(&v.to_be_bytes());
        }
        let mut file =
            aiff_core::header::build(&Spec::cd(0).format(), frames, audio.len() as u64).to_vec();
        file.extend(audio);
        let src = dir.path().join("tone.aif");
        fs::write(&src, file).unwrap();
        let plan = edit_core::plan(&[Clip::whole(&src)]).unwrap();
        let fade = edit_core::Fade::from_secs(0.6, 0.6, f64::from(rate));
        let out = dir.path().join(format!("o.{}", codec.extension()));
        let spec = ExportSpec {
            format: plan.format.clone(),
            codec,
            output: out.clone(),
            metadata: Metadata::default(),
        };
        run(
            &ffmpeg,
            &spec,
            &mut plan.reader_faded(fade),
            plan.total_bytes(),
            &mut |_, _| true,
        )
        .unwrap();
        let dec = Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&out)
            .args(["-ac", "1", "-ar", "44100", "-f", "s16le", "-"])
            .output()
            .unwrap();
        let s: Vec<f64> = dec
            .stdout
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| f64::from(i16::from_le_bytes(b)) / 32768.0)
            .collect();
        let rms = |a: usize, b: usize| {
            (s[a..b].iter().map(|x| x * x).sum::<f64>() / (b - a) as f64).sqrt()
        };
        let w = 4410;
        // Skip the codec's own priming/padding at the very edges: 20 ms in from each end.
        Some((
            rms(900, 900 + w),
            rms(s.len() / 2 - w / 2, s.len() / 2 + w / 2),
            rms(s.len() - 900 - w, s.len() - 900),
        ))
    }

    #[test]
    fn mp3_fades_in_and_out_while_the_middle_stays_loud() {
        let Some((start, middle, end)) = faded_levels(mp3(Mp3Mode::Cbr(256))) else {
            return;
        };
        assert!(middle > 0.3, "middle {middle}");
        assert!(start < middle * 0.25, "start {start} vs middle {middle}");
        assert!(end < middle * 0.25, "end {end} vs middle {middle}");
    }

    #[test]
    fn aac_fades_in_and_out_while_the_middle_stays_loud() {
        let Some((start, middle, end)) = faded_levels(aac("aac", AacMode::Cbr(192))) else {
            return;
        };
        assert!(
            start < middle * 0.25 && end < middle * 0.25,
            "{start} {middle} {end}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn apple_aac_gets_the_same_fades() {
        if crate::apple::usable_windows_dll().is_none() {
            return eprintln!("skipping: no Apple DLL");
        }
        let Some((start, middle, end)) = faded_levels(aac(APPLE_ENCODER, AacMode::Cbr(256))) else {
            return;
        };
        assert!(
            middle > 0.3 && start < middle * 0.25 && end < middle * 0.25,
            "{start} {middle} {end}"
        );
    }

    #[cfg(windows)]
    fn decoded_click_position(rate: u32, mode: AacMode) -> Option<(usize, usize)> {
        let (ffmpeg, _) = tools()?;
        crate::apple::usable_windows_dll()?;
        let click_at = rate as usize / 2; // 0.5 s
        let total = rate as usize * 2;
        let mut pcm = vec![0u8; total * 4];
        for i in 0..16 {
            let v: i16 = if i % 2 == 0 { 20000 } else { -20000 };
            for ch in 0..2 {
                let o = ((click_at + i) * 2 + ch) * 2;
                pcm[o..o + 2].copy_from_slice(&v.to_be_bytes());
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("click.m4a");
        let format = Format {
            channels: 2,
            bits: 16,
            sample_rate: f64::from(rate),
        };
        let spec = ExportSpec {
            format,
            codec: aac(APPLE_ENCODER, mode),
            output: out.clone(),
            metadata: Metadata::default(),
        };
        run(
            &ffmpeg,
            &spec,
            &mut &pcm[..],
            pcm.len() as u64,
            &mut |_, _| true,
        )
        .unwrap();
        let dec = Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&out)
            .args(["-ac", "1", "-f", "s16le", "-"])
            .output()
            .unwrap();
        let samples: Vec<i16> = dec
            .stdout
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| i16::from_le_bytes(b))
            .collect();
        let peak = samples
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| v.unsigned_abs())
            .map(|(i, _)| i)?;
        Some((peak, click_at))
    }

    /// The whole point of the delay handling: a click 0.5 s into the source must decode at 0.5 s,
    /// not 48 ms late as it did when the encoder delay was left in.
    #[cfg(windows)]
    #[test]
    fn apple_aac_audio_lands_where_it_was_recorded() {
        for (rate, mode) in [
            (44100, AacMode::Cbr(256)),
            (44100, AacMode::Vbr(3)),
            (48000, AacMode::Cbr(192)),
        ] {
            let Some((peak, expected)) = decoded_click_position(rate, mode.clone()) else {
                return eprintln!("skipping: need ffmpeg and Apple Application Support");
            };
            let error_ms = (peak as f64 - expected as f64).abs() / f64::from(rate) * 1000.0;
            assert!(error_ms < 2.0, "{rate} Hz {mode:?}: click off by {error_ms:.2} ms (peak {peak}, expected {expected})");
        }
    }

    #[test]
    fn extension_matches_container() {
        assert_eq!(mp3(Mp3Mode::Vbr(2)).extension(), "mp3");
        assert_eq!(aac("aac", AacMode::Cbr(128)).extension(), "m4a");
    }

    #[test]
    fn user_choice_beats_bundled_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("my-ffmpeg");
        fs::write(&user, b"").unwrap();
        let bundled = tempfile::tempdir().unwrap();
        fs::write(bundled.path().join(exe_name()), b"").unwrap();
        assert_eq!(
            locate_ffmpeg(Some(&user), &[bundled.path().to_path_buf()]),
            Some(user)
        );
    }

    #[test]
    fn bundled_is_used_when_no_user_choice() {
        let bundled = tempfile::tempdir().unwrap();
        let exe = bundled.path().join(exe_name());
        fs::write(&exe, b"").unwrap();
        assert_eq!(
            locate_ffmpeg(None, &[bundled.path().to_path_buf()]),
            Some(exe)
        );
    }

    #[test]
    fn a_missing_user_path_falls_through() {
        let bundled = tempfile::tempdir().unwrap();
        let exe = bundled.path().join(exe_name());
        fs::write(&exe, b"").unwrap();
        let got = locate_ffmpeg(
            Some(Path::new("nope/ffmpeg")),
            &[bundled.path().to_path_buf()],
        );
        assert_eq!(got, Some(exe));
    }

    // ---- integration: needs a real ffmpeg/ffprobe on PATH; skipped otherwise ----

    fn tools() -> Option<(PathBuf, PathBuf)> {
        let ffmpeg = locate_ffmpeg(None, &[])?;
        let probe = ffmpeg.with_file_name(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        probe.is_file().then_some((ffmpeg, probe))
    }

    fn probe_field(probe: &Path, file: &Path, entries: &str) -> String {
        let out = Command::new(probe)
            .args([
                "-v",
                "error",
                "-show_entries",
                entries,
                "-of",
                "default=nw=1:nk=1",
            ])
            .arg(file)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .replace("\r\n", "\n")
    }

    fn duration(probe: &Path, file: &Path) -> f64 {
        probe_field(probe, file, "format=duration")
            .parse()
            .unwrap_or_else(|_| panic!("ffprobe could not read {}", file.display()))
    }

    fn tag(probe: &Path, file: &Path, key: &str) -> String {
        probe_field(probe, file, &format!("format_tags={key}"))
    }

    fn export_fixture(
        kind: &str,
        codec: Codec,
        broken: bool,
    ) -> Option<(tempfile::TempDir, PathBuf, PathBuf)> {
        let (ffmpeg, probe) = tools().or_else(|| {
            eprintln!("skipping: ffmpeg not found");
            None
        })?;
        let dir = tempfile::tempdir().unwrap();
        let s = Spec::cd(44100 * 2); // 2 seconds
        let src = dir.path().join("in.aif");
        fs::write(
            &src,
            if broken {
                fixtures::broken(s, 1, 3)
            } else {
                fixtures::healthy(s, 1)
            },
        )
        .unwrap();
        let plan = edit_core::plan(&[Clip::whole(&src)]).unwrap();
        let out = dir.path().join(format!("out.{}", codec.extension()));
        let metadata = Metadata {
            title: "Test Title".into(),
            artist: "Test Artist".into(),
            comment: "hello".into(),
            date: "2025".into(),
            ..Default::default()
        };
        let spec = ExportSpec {
            format: plan.format.clone(),
            codec,
            output: out.clone(),
            metadata,
        };
        run(
            &ffmpeg,
            &spec,
            &mut plan.reader(),
            plan.total_bytes(),
            &mut |_, _| true,
        )
        .unwrap_or_else(|e| panic!("{kind}: {e}"));
        let d = duration(&probe, &out);
        assert!((d - 2.0).abs() < 0.15, "{kind}: duration {d}");
        Some((dir, out, probe))
    }

    #[test]
    fn exports_mp3_with_tags_through_a_real_ffmpeg() {
        let Some((_d, out, probe)) = export_fixture("mp3", mp3(Mp3Mode::Cbr(192)), false) else {
            return;
        };
        assert_eq!(tag(&probe, &out, "title"), "Test Title");
        assert_eq!(tag(&probe, &out, "artist"), "Test Artist");
        assert_eq!(tag(&probe, &out, "comment"), "hello");
    }

    #[test]
    fn mp3_cbr_vbr_and_abr_all_encode() {
        for (name, mode) in [("vbr", Mp3Mode::Vbr(2)), ("abr", Mp3Mode::Abr(160))] {
            export_fixture(name, mp3(mode), false);
        }
    }

    #[test]
    fn exports_aac_with_tags_through_a_real_ffmpeg() {
        let Some((_d, out, probe)) = export_fixture("aac", aac("aac", AacMode::Cbr(192)), false)
        else {
            return;
        };
        assert_eq!(tag(&probe, &out, "title"), "Test Title");
        assert_eq!(probe_field(&probe, &out, "stream=codec_name"), "aac");
    }

    #[test]
    fn a_broken_file_exports_without_being_repaired_first() {
        export_fixture("broken->mp3", mp3(Mp3Mode::Cbr(192)), true);
    }

    #[test]
    fn refuses_to_overwrite_existing_output() {
        let Some((ffmpeg, _)) = tools() else { return };
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("x.mp3");
        fs::write(&out, b"keep").unwrap();
        let spec = ExportSpec {
            format: Spec::cd(0).format(),
            codec: mp3(Mp3Mode::Cbr(128)),
            output: out.clone(),
            metadata: Metadata::default(),
        };
        let r = run(&ffmpeg, &spec, &mut std::io::empty(), 0, &mut |_, _| true);
        assert!(matches!(r, Err(Error::OutputExists(_))));
        assert_eq!(fs::read(&out).unwrap(), b"keep");
    }

    #[test]
    fn cancel_kills_ffmpeg_and_removes_the_output() {
        let Some((ffmpeg, _)) = tools() else { return };
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.aif");
        fs::write(&src, fixtures::healthy(Spec::cd(44100 * 30), 1)).unwrap();
        let plan = edit_core::plan(&[Clip::whole(&src)]).unwrap();
        let out = dir.path().join("o.mp3");
        let spec = ExportSpec {
            format: plan.format.clone(),
            codec: mp3(Mp3Mode::Cbr(128)),
            output: out.clone(),
            metadata: Metadata::default(),
        };
        let r = run(
            &ffmpeg,
            &spec,
            &mut plan.reader(),
            plan.total_bytes(),
            &mut |_, _| false,
        );
        assert!(matches!(r, Err(Error::Cancelled)));
        assert!(!out.exists());
    }

    #[test]
    fn a_bad_codec_surfaces_ffmpegs_message() {
        let Some((ffmpeg, _)) = tools() else { return };
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.aif");
        fs::write(&src, fixtures::healthy(Spec::cd(44100), 1)).unwrap();
        let plan = edit_core::plan(&[Clip::whole(&src)]).unwrap();
        let spec = ExportSpec {
            format: plan.format.clone(),
            codec: aac("no_such_encoder", AacMode::Cbr(128)),
            output: dir.path().join("o.m4a"),
            metadata: Metadata::default(),
        };
        let r = run(
            &ffmpeg,
            &spec,
            &mut plan.reader(),
            plan.total_bytes(),
            &mut |_, _| true,
        );
        let Err(Error::Ffmpeg { log, .. }) = r else {
            panic!("expected ffmpeg error, got {r:?}")
        };
        assert!(
            log.to_lowercase().contains("no_such_encoder")
                || log.to_lowercase().contains("unknown encoder"),
            "{log}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn apple_encoder_produces_a_playable_tagged_m4a_through_ffmpeg() {
        let (Some((ffmpeg, probe)), Some(_)) = (tools(), crate::apple::usable_windows_dll()) else {
            return eprintln!("skipping: need ffmpeg and Apple Application Support");
        };
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.aif");
        fs::write(&src, fixtures::healthy(Spec::cd(44100 * 4), 1)).unwrap();
        let plan = edit_core::plan(&[Clip::whole(&src)]).unwrap();
        for mode in [AacMode::Cbr(256), AacMode::Vbr(3)] {
            let out = dir
                .path()
                .join(format!("apple-{mode:?}.m4a").replace(['(', ')'], "-"));
            let metadata = Metadata {
                title: "Apple Test".into(),
                ..Default::default()
            };
            let spec = ExportSpec {
                format: plan.format.clone(),
                codec: aac(APPLE_ENCODER, mode.clone()),
                output: out.clone(),
                metadata,
            };
            run(
                &ffmpeg,
                &spec,
                &mut plan.reader(),
                plan.total_bytes(),
                &mut |_, _| true,
            )
            .unwrap_or_else(|e| panic!("{mode:?}: {e}"));
            let d = duration(&probe, &out);
            // Apple's encoder adds ~2112 samples (48 ms) of priming that ADTS cannot carry.
            assert!((d - 4.0).abs() < 0.15, "{mode:?}: duration {d}");
            assert_eq!(probe_field(&probe, &out, "stream=codec_name"), "aac");
            assert_eq!(tag(&probe, &out, "title"), "Apple Test");
            // Decoding the whole file must succeed.
            let dec = Command::new(&ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(&out)
                .args(["-f", "null", "-"])
                .output()
                .unwrap();
            assert!(
                dec.status.success(),
                "{}",
                String::from_utf8_lossy(&dec.stderr)
            );
        }
    }
}
