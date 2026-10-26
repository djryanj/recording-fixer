//! Against the real recordings in the repo root (gitignored): `cargo test -p dynamics -- --ignored --nocapture`.

use aiff_core::diagnose_path;
use dynamics::maximizer::measure_with;
use dynamics::pcm::PcmFormat;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Instant;

fn sample(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(name)
}

/// How long the wait in front of the loudness graph and preview is, measured the way the app does it
/// (a megabyte read buffer, every batch of steps published as it is made).
#[test]
#[ignore = "needs Serato Recording 1.aif in the repo root"]
fn measuring_the_three_hour_recording_is_quick_and_the_graph_can_start_early() {
    let path = sample("Serato Recording 1.aif");
    let d = diagnose_path(&path).unwrap();
    let fmt = PcmFormat {
        bits: d.format.bits,
        channels: d.format.channels,
        rate: d.format.sample_rate,
    };
    let mut f = std::fs::File::open(&path).unwrap();
    f.seek(SeekFrom::Start(d.audio_start)).unwrap();
    let started = Instant::now();
    let (mut first_steps, mut published) = (None, 0usize);
    let profile = measure_with(
        BufReader::with_capacity(1 << 20, f).take(d.frames * fmt.frame_bytes() as u64),
        fmt,
        d.frames,
        &mut |_, _, _| true,
        &mut |k, _| {
            first_steps.get_or_insert(started.elapsed());
            published += k.len();
        },
    )
    .unwrap();
    let took = started.elapsed();
    let secs = d.frames as f64 / fmt.rate;
    eprintln!(
        "measured {secs:.0} s in {:.1} s ({:.0}x realtime); first steps after {:.0} ms; integrated {:.1} LUFS",
        took.as_secs_f64(),
        secs / took.as_secs_f64(),
        first_steps.unwrap().as_secs_f64() * 1000.0,
        profile.integrated()
    );
    assert!(published >= profile.hops() - 1);
    // Minutes became seconds: well under a minute even on a slow disk and a busy machine.
    assert!(
        took.as_secs_f64() < 60.0,
        "took {:.0} s",
        took.as_secs_f64()
    );
    assert!(
        first_steps.unwrap().as_millis() < 1000,
        "the graph should start filling within a second"
    );
}
