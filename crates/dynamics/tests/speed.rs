//! A guard against measuring becoming slow again. Measuring a recording is the wait in front of
//! the loudness preview, so it has to run far faster than real time even in a development build
//! (the workspace Cargo.toml optimises this crate for that; unoptimised it ran at about 40x).

use dynamics::maximizer::measure;
use dynamics::pcm::PcmFormat;
use std::time::Instant;

#[test]
fn five_minutes_of_stereo_are_measured_in_a_few_seconds() {
    let fmt = PcmFormat {
        bits: 16,
        channels: 2,
        rate: 44_100.0,
    };
    let frames = 300 * 44_100u64;
    let mut pcm = Vec::with_capacity(frames as usize * 4);
    for i in 0..frames {
        let v = ((i as f64 * 0.05).sin() * 9000.0) as i16;
        pcm.extend_from_slice(&v.to_be_bytes());
        pcm.extend_from_slice(&v.to_be_bytes());
    }
    let started = Instant::now();
    let profile = measure(&pcm[..], fmt, frames, &mut |_, _, _| true).unwrap();
    let took = started.elapsed().as_secs_f64();
    assert_eq!(profile.hops(), 3000);
    // About 0.4 s optimised; 8 s unoptimised. Anything past 4 s means the speed has regressed.
    assert!(took < 4.0, "measuring 300 s of audio took {took:.1} s");
}
