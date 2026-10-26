//! Run the maximizer on an AIFF file from the command line, to listen to the result and read the numbers.
//!   cargo run --release -p dynamics --example maximize -- IN.aif OUT.aif [--lufs -14 | --peak] [--ceiling -1]
//!                                                         [--level off|gentle|medium|strong] [--no-precise]
use aiff_core::{diagnose_path, header};
use dynamics::leveler::Leveling;
use dynamics::maximizer::{prepare, Maximizer, Mode, Params};
use dynamics::pcm::PcmFormat;
use std::io::{Read, Seek, SeekFrom, Write};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let (input, output) = (&args[0], &args[1]);
    let ceiling: f64 = flag("--ceiling").map_or(-1.0, |v| v.parse().unwrap());
    let mode = if args.iter().any(|a| a == "--peak") {
        Mode::Peak
    } else {
        Mode::Loudness(flag("--lufs").map_or(-14.0, |v| v.parse().unwrap()))
    };
    let leveling = match flag("--level").as_deref().unwrap_or("medium") {
        "off" => Leveling::OFF,
        "gentle" => Leveling::gentle(),
        "strong" => Leveling::strong(),
        _ => Leveling::medium(),
    };
    let params = Params {
        mode,
        ceiling_db: ceiling,
        leveling,
        precise: !args.iter().any(|a| a == "--no-precise"),
    };

    let d = diagnose_path(std::path::Path::new(input)).unwrap();
    let fmt = PcmFormat {
        bits: d.format.bits,
        channels: d.format.channels,
        rate: d.format.sample_rate,
    };
    let open = || -> std::io::Result<std::io::Take<std::fs::File>> {
        let mut f = std::fs::File::open(input)?;
        f.seek(SeekFrom::Start(d.audio_start))?;
        Ok(f.take(d.frames * fmt.frame_bytes() as u64))
    };
    let t0 = Instant::now();
    let mut last = Instant::now();
    let prepared = prepare(open, fmt, d.frames, &params, &mut |phase, done, total| {
        if last.elapsed().as_secs() >= 2 {
            eprintln!("{phase:?}: {:.0}%", 100.0 * done as f64 / total as f64);
            last = Instant::now();
        }
        true
    })
    .unwrap();
    eprintln!("prepared in {:.1} s", t0.elapsed().as_secs_f64());

    let t1 = Instant::now();
    let mut m = Maximizer::new(open().unwrap(), &prepared);
    let handle = m.report_handle();
    let mut out = std::io::BufWriter::new(std::fs::File::create(output).unwrap());
    let bytes = d.frames * fmt.frame_bytes() as u64;
    out.write_all(&header::build(&d.format, d.frames as u32, bytes))
        .unwrap();
    std::io::copy(&mut m, &mut out).unwrap();
    out.flush().unwrap();
    eprintln!("processed in {:.1} s", t1.elapsed().as_secs_f64());

    let r = handle.lock().unwrap().take().unwrap();
    println!("                 integrated    range   true peak");
    println!(
        "  before       {:>8.1} LUFS {:>5.1} LU {:>7.2} dBTP",
        r.input.lufs, r.input.range_lu, r.input.true_peak_db
    );
    println!(
        "  after        {:>8.1} LUFS {:>5.1} LU {:>7.2} dBTP",
        r.output.lufs, r.output.range_lu, r.output.true_peak_db
    );
    println!(
        "  target {:?}, ceiling {:.1} dBTP",
        r.target_lufs, r.ceiling_db
    );
    println!(
        "  static gain {:+.1} dB; leveller moved it between {:+.1} and {:+.1} dB",
        r.static_gain_db, r.gain_min_db, r.gain_max_db
    );
    println!(
        "  limiter: deepest {:.1} dB, average {:.2} dB, working {:.1}% of the time",
        r.limiter.max_reduction_db,
        r.limiter.mean_reduction_db,
        100.0 * r.limiter.active_fraction
    );
}
