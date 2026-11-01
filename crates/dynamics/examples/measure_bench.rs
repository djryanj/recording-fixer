//! How long does measuring take? Build with and without --release to see the difference a profile makes.
//!   cargo run -p dynamics --example measure_bench -- "Serato Recording 1.aif" [seconds]
use aiff_core::diagnose_path;
use dynamics::maximizer::measure;
use dynamics::pcm::PcmFormat;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("path to an .aif");
    let limit: Option<f64> = std::env::args().nth(2).map(|s| s.parse().unwrap());
    let d = diagnose_path(std::path::Path::new(&path)).unwrap();
    let fmt = PcmFormat {
        bits: d.format.bits,
        channels: d.format.channels,
        rate: d.format.sample_rate,
    };
    let frames = limit.map_or(d.frames, |s| ((s * fmt.rate) as u64).min(d.frames));
    let mut f = std::fs::File::open(&path).unwrap();
    f.seek(SeekFrom::Start(d.audio_start)).unwrap();
    let t = Instant::now();
    let p = measure(
        BufReader::with_capacity(1 << 20, f).take(frames * fmt.frame_bytes() as u64),
        fmt,
        frames,
        &mut |_, _, _| true,
    )
    .unwrap();
    let took = t.elapsed().as_secs_f64();
    let secs = frames as f64 / fmt.rate;
    println!(
        "measured {:.0} s of audio in {took:.2} s = {:.0}x realtime ({} steps, integrated {:.1} LUFS)",
        secs, secs / took, p.hops(), p.integrated()
    );
}
