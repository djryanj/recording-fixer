//! `device_check [device-id] [out-pair] [file.aif]`
//!
//! Lists the output devices, then plays 1.5 s of a file very quietly (volume 5%) through the
//! chosen one, so you can confirm real audio output without a loud surprise.

use edit_core::Clip;
use playback::DeviceSelection;
use std::time::{Duration, Instant};

fn main() {
    let mut args = std::env::args().skip(1);
    let id = args.next().filter(|s| s != "default");
    let pair: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let file = args
        .next()
        .unwrap_or_else(|| "tests/fixtures/serato-broken-head.aif".into());

    let pb = playback::start_system();
    println!("Output devices:");
    for d in pb.devices() {
        println!(
            "  {}{:<48} {:<8} {}",
            if d.is_default { "*" } else { " " },
            d.name,
            d.host,
            d.id
        );
    }
    let sel = DeviceSelection {
        id,
        out_pair: pair,
        buffer_frames: None,
    };
    match pb.set_device(sel) {
        Ok(info) => println!(
            "\nOpened: {} via {} at {} Hz, {} ch (pair {}), buffer {:?}",
            info.name,
            info.host,
            info.rate,
            info.channels,
            info.out_pair + 1,
            info.buffer_frames
        ),
        Err(e) => return println!("\nCould not open device: {e}"),
    }
    let info = pb.load(vec![Clip::whole(&file)], 0.0).expect("load file");
    println!(
        "Loaded {file}: {:.1} s, {} Hz, {}-bit, {} ch",
        info.duration_secs, info.rate, info.bits, info.channels
    );
    pb.set_volume(0.05);
    pb.play();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(1500) {
        std::thread::sleep(Duration::from_millis(250));
        let s = pb.state();
        println!(
            "  t={:.2}s playing={} peak L/R={:.3}/{:.3} underruns={}",
            s.position_secs, s.playing, s.peak_l, s.peak_r, s.underruns
        );
    }
    pb.pause();
    println!("Done. Position {:.2}s", pb.state().position_secs);
}
