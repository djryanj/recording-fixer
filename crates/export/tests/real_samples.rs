//! End-to-end checks on the real sample recordings. Run with
//! `cargo test -p export -- --ignored` (needs the files in the repo root and ffmpeg/ffprobe on PATH).

use edit_core::Clip;
use export::{locate_ffmpeg, run, AacMode, Codec, ExportSpec, Metadata, Mp3Mode, Mp3Options};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ffprobe_duration(file: &Path) -> f64 {
    let ffmpeg = locate_ffmpeg(None, &[]).expect("ffmpeg on PATH");
    let probe = ffmpeg.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    let out = Command::new(probe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("ffprobe duration")
}

#[test]
#[ignore = "needs the full sample files"]
fn repaired_sample_5_is_readable_by_ffprobe_and_matches_expected_length() {
    let dir = tempfile::tempdir().unwrap();
    let fixed = dir.path().join("fixed.aif");
    aiff_core::repair_copy(
        &root().join("Serato Recording 5.aif"),
        &fixed,
        &mut |_, _| true,
    )
    .unwrap();
    let d = ffprobe_duration(&fixed);
    assert!((d - 55_121_152.0 / 48000.0).abs() < 0.01, "duration {d}");
}

#[test]
#[ignore = "needs the full sample files"]
fn broken_sample_5_exports_straight_to_mp3_and_aac() {
    let ffmpeg = locate_ffmpeg(None, &[]).expect("ffmpeg on PATH");
    let plan = edit_core::plan(&[Clip::whole(root().join("Serato Recording 5.aif"))]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut codecs = vec![
        Codec::Mp3(Mp3Options::new(Mp3Mode::Cbr(192))),
        Codec::Aac {
            encoder: "aac".into(),
            mode: AacMode::Cbr(192),
        },
    ];
    if cfg!(windows) && export::apple::usable_windows_dll().is_some() {
        codecs.push(Codec::Aac {
            encoder: export::APPLE_ENCODER.into(),
            mode: AacMode::Vbr(4),
        });
    }
    for (i, codec) in codecs.into_iter().enumerate() {
        let out = dir.path().join(format!("out{i}.{}", codec.extension()));
        let spec = ExportSpec {
            format: plan.format.clone(),
            codec,
            output: out.clone(),
            metadata: Metadata::default(),
        };
        run(
            &ffmpeg,
            &spec,
            &mut plan.reader(),
            plan.total_bytes(),
            &mut |_, _| true,
        )
        .unwrap();
        let d = ffprobe_duration(&out);
        assert!(
            (d - plan.duration_secs()).abs() < 0.2,
            "{}: {d}",
            out.display()
        );
    }
}
