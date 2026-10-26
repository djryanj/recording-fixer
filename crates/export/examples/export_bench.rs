//! `export_bench <in.aif> <out> <mp3|aac|apple>`: encode a whole file and print timing.
//! Peak memory is measured by the caller (see docs/PERF.md).

use edit_core::Clip;
use export::{
    locate_ffmpeg, run, AacMode, Codec, ExportSpec, Metadata, Mp3Mode, Mp3Options, APPLE_ENCODER,
};
use std::time::Instant;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let input = args
        .next()
        .expect("usage: export_bench <in.aif> <out> <mp3|aac|apple>");
    let output = args.next().expect("output path");
    let which = args.next().expect("codec").to_string_lossy().into_owned();
    let codec = match which.as_str() {
        "mp3" => Codec::Mp3(Mp3Options::new(Mp3Mode::Cbr(320))),
        "aac" => Codec::Aac {
            encoder: "aac".into(),
            mode: AacMode::Cbr(256),
        },
        _ => Codec::Aac {
            encoder: APPLE_ENCODER.into(),
            mode: AacMode::Cbr(256),
        },
    };
    let ffmpeg = locate_ffmpeg(None, &[]).expect("ffmpeg on PATH");
    let plan = edit_core::plan(&[Clip::whole(input)]).expect("plan");
    let spec = ExportSpec {
        format: plan.format.clone(),
        codec,
        output: output.into(),
        metadata: Metadata::default(),
    };
    let started = Instant::now();
    run(
        &ffmpeg,
        &spec,
        &mut plan.reader(),
        plan.total_bytes(),
        &mut |_, _| true,
    )
    .expect("export");
    let secs = started.elapsed().as_secs_f64();
    println!(
        "{which}: {:.1} s for {:.0} s of audio ({:.0}x realtime)",
        secs,
        plan.duration_secs(),
        plan.duration_secs() / secs
    );
}
