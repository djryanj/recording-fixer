//! `peaks_bench <file.aif>`: build peaks for a whole file and print timing and sizes.
//! Peak memory is measured by the caller (see docs/PERF.md).

use std::time::Instant;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .expect("usage: peaks_bench <file.aif>");
    let started = Instant::now();
    let p = peaks::build_for_file(std::path::Path::new(&path), &mut |_, _| true).expect("build");
    let secs = started.elapsed().as_secs_f64();
    let bytes: usize = p.levels.iter().map(Vec::len).sum();
    println!(
        "{} frames, {} levels, {:.1} MB of peaks, built in {secs:.2} s",
        p.frames,
        p.levels.len(),
        bytes as f64 / 1e6
    );
    let q = Instant::now();
    let cols = p.columns(0, p.frames, 2000);
    println!(
        "full-file overview (2000 columns): {} bytes in {:?}",
        cols.len(),
        q.elapsed()
    );
    let q = Instant::now();
    let cols = p.columns(p.frames / 2, p.frames / 2 + 44100 * 10, 1500);
    println!(
        "10 s zoom (1500 columns): {} bytes in {:?}",
        cols.len(),
        q.elapsed()
    );
}
