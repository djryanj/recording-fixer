//! Benchmark helper: `join_bench <out.aif> <in1.aif> [in2.aif ...]` joins whole files and
//! prints elapsed time and throughput. Peak memory is measured by the caller (see docs/PERF.md).

use edit_core::{plan, Clip};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let out = PathBuf::from(args.next().expect("usage: join_bench <out> <in>..."));
    let clips: Vec<Clip> = args.map(Clip::whole).collect();
    let started = Instant::now();
    let plan = plan(&clips).expect("plan");
    let bytes = plan.total_bytes();
    plan.write_aiff(&out, &mut |_, _| true).expect("write");
    let secs = started.elapsed().as_secs_f64();
    println!(
        "{:.1} s for {:.2} GB ({:.0} MB/s)",
        secs,
        bytes as f64 / 1e9,
        bytes as f64 / 1e6 / secs
    );
}
