//! Measure a recording and point at the stretches worth testing the maximizer on.
//!   cargo run --release -p dynamics --example analyze -- "Serato Recording 1.aif"
use aiff_core::diagnose_path;
use dynamics::loudness::{Analyzer, Profile};
use dynamics::pcm::{decode, PcmFormat};
use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

fn stamp(secs: f64) -> String {
    let s = secs.round() as u64;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn db(lin: f64) -> f64 {
    20.0 * lin.max(1e-9).log10()
}

fn main() {
    let path = std::env::args().nth(1).expect("path to an .aif");
    let d = diagnose_path(std::path::Path::new(&path)).unwrap();
    let fmt = PcmFormat {
        bits: d.format.bits,
        channels: d.format.channels,
        rate: d.format.sample_rate,
    };
    let mut f = std::fs::File::open(&path).unwrap();
    f.seek(SeekFrom::Start(d.audio_start)).unwrap();
    let mut left = d.frames * fmt.frame_bytes() as u64;
    let mut an = Analyzer::new(fmt.rate, usize::from(fmt.channels), true);
    let (mut buf, mut samples) = (vec![0u8; 1 << 20], Vec::new());
    let t0 = Instant::now();
    while left > 0 {
        let want = buf.len().min(left as usize) / fmt.frame_bytes() * fmt.frame_bytes();
        f.read_exact(&mut buf[..want]).unwrap();
        samples.clear();
        decode(fmt, &buf[..want], &mut samples);
        an.push(&samples);
        left -= want as u64;
    }
    let p: Profile = an.finish();
    let took = t0.elapsed().as_secs_f64();
    let hop = p.hop_secs();
    println!(
        "{path}: {} Hz {}-bit {} ch, {} ({:.0} s)\nanalysis took {took:.1} s = {:.0}x realtime",
        fmt.rate,
        fmt.bits,
        fmt.channels,
        stamp(p.duration_secs()),
        p.duration_secs(),
        p.duration_secs() / took
    );
    println!(
        "integrated {:.1} LUFS, range {:.1} LU, true peak {:.2} dBTP",
        p.integrated(),
        p.loudness_range(),
        db(f64::from(p.peak_max()))
    );

    let n = p.hops();
    let st: Vec<f64> = (0..n).map(|i| p.short_term(i)).collect();
    // Local reference: mean short-term loudness over +-3 minutes of audible material.
    let r = (180.0 / hop) as isize;
    let local: Vec<f64> = (0..n as isize)
        .map(|i| {
            let (a, b) = ((i - r).max(0) as usize, ((i + r) as usize).min(n));
            let v: Vec<f64> = st[a..b].iter().copied().filter(|&l| l > -50.0).collect();
            v.iter().sum::<f64>() / v.len().max(1) as f64
        })
        .collect();
    let dev: Vec<f64> = (0..n).map(|i| st[i] - local[i]).collect();
    let bass: Vec<f64> = (0..n)
        .map(|i| (p.low_ms[i] / (p.full_ms[i] + 1e-12)).min(1.0))
        .collect();

    // Runs of consecutive steps satisfying `pred`, at least `min_s` long.
    let runs = |pred: &dyn Fn(usize) -> bool, min_s: f64| -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            if pred(i) {
                let s = i;
                while i < n && pred(i) {
                    i += 1;
                }
                if (i - s) as f64 * hop >= min_s {
                    out.push((s, i));
                }
            } else {
                i += 1;
            }
        }
        out
    };
    let describe = |label: &str,
                    mut v: Vec<(usize, usize)>,
                    score: &dyn Fn(usize, usize) -> f64| {
        v.sort_by(|a, b| score(b.0, b.1).partial_cmp(&score(a.0, a.1)).unwrap());
        println!("\n{label}");
        for (a, b) in v.into_iter().take(6) {
            let m = (a + b) / 2;
            println!(
                "  {} - {}  ({:>3.0} s)  loudness {:>6.1} LUFS ({:+.1} vs surroundings), bass {:>3.0}% of energy, peak {:.1} dBFS",
                stamp(a as f64 * hop), stamp(b as f64 * hop), (b - a) as f64 * hop, st[m], dev[m], bass[m] * 100.0,
                db(f64::from(p.peak[a..b].iter().copied().fold(0.0, f32::max)))
            );
        }
    };
    let mean = |a: usize, b: usize, v: &[f64]| v[a..b].iter().sum::<f64>() / (b - a) as f64;

    describe(
        "LOUD stretches (candidate beatmixes / hot sections: gain should come DOWN)",
        runs(&|i| dev[i] > 2.0, 15.0),
        &|a, b| mean(a, b, &dev),
    );
    describe(
        "QUIET stretches that are still music (a track too low: gain should come UP)",
        runs(&|i| dev[i] < -2.0 && st[i] > -45.0, 30.0),
        &|a, b| -mean(a, b, &dev),
    );
    describe(
        "DEEP quiet passages (breakdowns / intros: the energy arc, should NOT be flattened)",
        runs(&|i| dev[i] < -9.0 && st[i] > -60.0, 8.0),
        &|a, b| -mean(a, b, &dev),
    );
    describe(
        "BASS-HEAVY stretches (most energy below 150 Hz)",
        runs(&|i| bass[i] > 0.6 && st[i] > -30.0, 15.0),
        &|a, b| mean(a, b, &bass),
    );
    // Peak-to-loudness ratio over 20 s blocks: where the limiter will have the most to do.
    let block = (20.0 / hop) as usize;
    let plr: Vec<(usize, usize)> = (0..n)
        .step_by(block)
        .map(|a| (a, (a + block).min(n)))
        .filter(|&(a, b)| b - a == block)
        .collect();
    describe("HIGH PEAK-TO-LOUDNESS (sharp transients: the limiter works hardest here once the level is raised)", plr, &|a, b| {
        db(f64::from(p.peak[a..b].iter().copied().fold(0.0, f32::max))) - mean(a, b, &st)
    });
    let hot = runs(&|i| p.peak[i] >= 0.999, 0.1);
    println!(
        "\nsteps touching full scale (clipped/inter-sample peaks): {}",
        hot.len()
    );
    for (a, _) in hot.iter().take(6) {
        println!("  {}", stamp(*a as f64 * hop));
    }

    println!("\nTWO-MINUTE OVERVIEW  (short-term LUFS mean, loudest 100 ms step is 'max', bass share, peak)");
    let step = (120.0 / hop) as usize;
    for a in (0..n).step_by(step) {
        let b = (a + step).min(n);
        let m = mean(a, b, &st.iter().map(|&l| l.max(-80.0)).collect::<Vec<_>>());
        let top = st[a..b].iter().copied().fold(-99.0, f64::max);
        println!(
            "  {}  mean {:>6.1}  max {:>6.1}  bass {:>3.0}%  peak {:>5.1} dBFS",
            stamp(a as f64 * hop),
            m,
            top,
            mean(a, b, &bass) * 100.0,
            db(f64::from(p.peak[a..b].iter().copied().fold(0.0, f32::max)))
        );
    }
}
