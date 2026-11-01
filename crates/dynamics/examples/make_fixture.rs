//! Render the made-up beeps set to `tests/fixtures/beeps-set.aif` and print what it measures.
//!   cargo run --release -p dynamics --example make_fixture [-- --write]
use aiff_core::{header, Format};
use dynamics::loudness::Analyzer;
use dynamics::synth::{beeps_set, to_pcm16, RATE, SECTIONS};

fn main() {
    let samples = beeps_set();
    let mut an = Analyzer::new(RATE, 2, true);
    an.push(&samples);
    let p = an.finish();
    println!(
        "integrated {:.1} LUFS, range {:.1} LU, true peak {:.2} dBTP",
        p.integrated(),
        p.loudness_range(),
        20.0 * f64::from(p.peak_max()).log10()
    );
    for (name, a, b) in SECTIONS {
        let (i0, i1) = (
            ((a + 1.5) / 0.1) as usize,
            (((b - 1.5) / 0.1) as usize).max(((a + 1.5) / 0.1) as usize + 1),
        );
        let mid = (i0..i1)
            .max_by(|&x, &y| p.short_term(x).partial_cmp(&p.short_term(y)).unwrap())
            .unwrap();
        let peak = p.peak[(a * 10.0) as usize..(b * 10.0) as usize]
            .iter()
            .copied()
            .fold(0.0, f32::max);
        println!(
            "  {name:<20} {:>6.1} LUFS-S (loudest)  peak {:>6.1} dBFS  bass {:>3.0}%",
            p.short_term(mid),
            20.0 * f64::from(peak).max(1e-9).log10(),
            100.0 * p.low_ms[mid] / p.full_ms[mid].max(1e-12)
        );
    }
    if std::env::args().any(|a| a == "--write") {
        let pcm = to_pcm16(&samples);
        let fmt = Format {
            channels: 2,
            bits: 16,
            sample_rate: RATE,
        };
        let mut file = header::build(&fmt, (pcm.len() / 4) as u32, pcm.len() as u64).to_vec();
        file.extend(&pcm);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/beeps-set.aif");
        std::fs::write(&path, &file).unwrap();
        println!("wrote {} ({} bytes)", path.display(), file.len());
    }
}
