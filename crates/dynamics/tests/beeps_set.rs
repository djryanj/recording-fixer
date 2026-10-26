//! The whole maximizer against the committed made-up DJ set (`tests/fixtures/beeps-set.aif`, the
//! output of `dynamics::synth`). Each test says what a good result looks like for one of the set's
//! stretches; see the table at the top of `src/synth.rs`.

use aiff_core::diagnose_path;
use dynamics::leveler::Leveling;
use dynamics::loudness::Analyzer;
use dynamics::maximizer::{measure, measure_with, prepare, Error, Maximizer, Mode, Params, Report};
use dynamics::pcm::{decode, PcmFormat};
use dynamics::synth::{beeps_set, to_pcm16, RATE, SECTIONS};
use std::io::Read;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/beeps-set.aif")
}

fn load() -> (PcmFormat, Vec<u8>) {
    let d = diagnose_path(&fixture()).unwrap();
    let mut bytes = std::fs::read(fixture()).unwrap();
    let start = d.audio_start as usize;
    bytes.drain(..start);
    bytes.truncate(d.frames as usize * 4);
    (
        PcmFormat {
            bits: 16,
            channels: 2,
            rate: d.format.sample_rate,
        },
        bytes,
    )
}

fn run(fmt: PcmFormat, pcm: &[u8], params: &Params) -> (Vec<u8>, Report) {
    let frames = (pcm.len() / fmt.frame_bytes()) as u64;
    let prepared = prepare(|| Ok(pcm), fmt, frames, params, &mut |_, _, _| true).unwrap();
    let mut m = Maximizer::new(pcm, &prepared);
    let handle = m.report_handle();
    let mut out = Vec::new();
    m.read_to_end(&mut out).unwrap();
    let report = handle.lock().unwrap().take().unwrap();
    (out, report)
}

fn streaming(ceiling_db: f64, lufs: f64, leveling: Leveling) -> Params {
    Params {
        mode: Mode::Loudness(lufs),
        ceiling_db,
        leveling,
        precise: true,
    }
}

/// Loudest short-term loudness inside each named stretch (ignoring its first and last 1.5 s,
/// which are blended with the neighbours by the 3 s window).
fn by_section(fmt: PcmFormat, pcm: &[u8]) -> Vec<(&'static str, f64)> {
    let mut an = Analyzer::new(fmt.rate, usize::from(fmt.channels), false);
    let mut f = Vec::new();
    decode(fmt, pcm, &mut f);
    an.push(&f);
    let p = an.finish();
    SECTIONS
        .iter()
        .map(|&(name, a, b)| {
            let (i0, i1) = (
                ((a + 1.5) * 10.0) as usize,
                (((b - 1.5) * 10.0) as usize).max(((a + 1.5) * 10.0) as usize + 1),
            );
            (
                name,
                (i0..i1).map(|i| p.short_term(i)).fold(f64::MIN, f64::max),
            )
        })
        .collect()
}

fn level(sections: &[(&str, f64)], name: &str) -> f64 {
    sections.iter().find(|s| s.0 == name).unwrap().1
}

#[test]
fn the_committed_file_is_what_the_generator_makes() {
    let (_, committed) = load();
    let fresh = to_pcm16(&beeps_set());
    assert_eq!(committed.len(), fresh.len());
    // Float maths may differ in the last bit between platforms: allow one step either way.
    let worst = committed
        .chunks(2)
        .zip(fresh.chunks(2))
        .map(|(a, b)| {
            (i32::from(i16::from_be_bytes([a[0], a[1]]))
                - i32::from(i16::from_be_bytes([b[0], b[1]])))
            .abs()
        })
        .max()
        .unwrap();
    assert!(worst <= 2, "committed fixture differs from the generator by {worst} steps; regenerate it with the make_fixture example");
    assert_eq!(RATE, 22_050.0);
}

#[test]
fn it_lands_on_the_target_loudness_without_crossing_the_ceiling() {
    let (fmt, pcm) = load();
    let (out, r) = run(fmt, &pcm, &streaming(-1.0, -14.0, Leveling::medium()));
    assert_eq!(out.len(), pcm.len(), "same format and length");
    assert!(
        (r.output.lufs + 14.0).abs() < 0.3,
        "integrated {:.2}",
        r.output.lufs
    );
    assert!(
        r.output.true_peak_db <= -0.95,
        "true peak {:.2}",
        r.output.true_peak_db
    );
    assert!(
        r.output.range_lu < r.input.range_lu,
        "range {} -> {}",
        r.input.range_lu,
        r.output.range_lu
    );
}

#[test]
fn a_beatmix_and_a_quiet_track_are_pulled_toward_the_rest_of_the_set() {
    let (fmt, pcm) = load();
    let before = by_section(fmt, &pcm);
    let (out, _) = run(fmt, &pcm, &streaming(-1.0, -14.0, Leveling::medium()));
    let after = by_section(fmt, &out);
    let beat = |s: &[(&str, f64)]| level(s, "beatmix A+B") - level(s, "A alone");
    let low = |s: &[(&str, f64)]| level(s, "A alone") - level(s, "B alone, 5 dB low");
    eprintln!("before {before:?}\nafter  {after:?}");
    assert!(
        beat(&after) < beat(&before) - 1.5,
        "beatmix {:.1} -> {:.1} LU above A",
        beat(&before),
        beat(&after)
    );
    assert!(
        low(&after) < low(&before) - 1.0,
        "quiet track {:.1} -> {:.1} LU below A",
        low(&before),
        low(&after)
    );
}

#[test]
fn the_breakdown_is_still_a_breakdown() {
    let (fmt, pcm) = load();
    let before = by_section(fmt, &pcm);
    let (out, _) = run(fmt, &pcm, &streaming(-1.0, -14.0, Leveling::medium()));
    let after = by_section(fmt, &out);
    let depth = |s: &[(&str, f64)]| level(s, "A alone") - level(s, "breakdown");
    assert!(
        depth(&after) > depth(&before) * 0.6,
        "breakdown depth {:.1} -> {:.1} LU",
        depth(&before),
        depth(&after)
    );
}

#[test]
fn stronger_leveling_flattens_more_and_off_flattens_nothing() {
    let (fmt, pcm) = load();
    let spread = |lv: Leveling| {
        let (out, _) = run(fmt, &pcm, &streaming(-1.0, -14.0, lv));
        let s = by_section(fmt, &out);
        level(&s, "beatmix A+B") - level(&s, "B alone, 5 dB low")
    };
    let (off, gentle, medium, strong) = (
        spread(Leveling::OFF),
        spread(Leveling::gentle()),
        spread(Leveling::medium()),
        spread(Leveling::strong()),
    );
    assert!(
        off > gentle && gentle > medium && medium > strong,
        "{off:.1} {gentle:.1} {medium:.1} {strong:.1}"
    );
}

#[test]
fn silence_stays_exactly_silent() {
    let (fmt, pcm) = load();
    let (out, _) = run(fmt, &pcm, &streaming(-1.0, -14.0, Leveling::strong()));
    let from = (38.4 * RATE) as usize * 4;
    let to = (39.6 * RATE) as usize * 4;
    assert!(out[from..to].iter().all(|&b| b == 0));
}

#[test]
fn audio_that_needs_nothing_comes_back_bit_for_bit() {
    let (fmt, pcm) = load();
    // Target its own loudness, no leveling, a ceiling nothing reaches: the chain must do nothing.
    let mut an = Analyzer::new(fmt.rate, 2, false);
    let mut f = Vec::new();
    decode(fmt, &pcm, &mut f);
    an.push(&f);
    let own = an.finish().integrated();
    let params = Params {
        mode: Mode::Loudness(own),
        ceiling_db: 6.0,
        leveling: Leveling::OFF,
        precise: false,
    };
    let (out, _) = run(fmt, &pcm, &params);
    assert!(out == pcm, "output differs from input");
}

#[test]
fn peak_mode_puts_the_loudest_peak_on_the_ceiling() {
    let (fmt, pcm) = load();
    let params = Params {
        mode: Mode::Peak,
        ceiling_db: -0.1,
        leveling: Leveling::OFF,
        precise: false,
    };
    let (_, r) = run(fmt, &pcm, &params);
    assert!(
        r.output.true_peak_db <= -0.1 + 0.02 && r.output.true_peak_db > -0.4,
        "{:.2}",
        r.output.true_peak_db
    );
    assert!(
        r.limiter.max_reduction_db < 0.5,
        "peak mode should not need limiting"
    );
}

#[test]
fn twenty_four_bit_audio_stays_twenty_four_bit() {
    let (fmt, pcm) = load();
    let pcm24: Vec<u8> = pcm.chunks(2).flat_map(|s| [s[0], s[1], 0]).collect();
    let fmt24 = PcmFormat { bits: 24, ..fmt };
    let (out, r) = run(fmt24, &pcm24, &streaming(-1.0, -14.0, Leveling::medium()));
    assert_eq!(out.len(), pcm24.len());
    assert!((r.output.lufs + 14.0).abs() < 0.3);
}

#[test]
fn it_can_be_cancelled_while_measuring() {
    let (fmt, pcm) = load();
    let frames = (pcm.len() / 4) as u64;
    let params = streaming(-1.0, -14.0, Leveling::medium());
    let result = prepare(|| Ok(&pcm[..]), fmt, frames, &params, &mut |_, done, _| {
        done < 100_000
    });
    assert!(matches!(result, Err(Error::Cancelled)));
}

#[test]
fn a_source_that_ends_early_is_an_error_not_silence() {
    let (fmt, pcm) = load();
    let frames = (pcm.len() / 4) as u64;
    let params = streaming(-1.0, -14.0, Leveling::medium());
    let prepared = prepare(|| Ok(&pcm[..]), fmt, frames, &params, &mut |_, _, _| true).unwrap();
    let mut m = Maximizer::new(&pcm[..pcm.len() / 2], &prepared);
    let mut out = Vec::new();
    assert!(m.read_to_end(&mut out).is_err());
}

#[test]
fn reading_in_odd_sized_pieces_gives_the_same_audio() {
    let (fmt, pcm) = load();
    let frames = (pcm.len() / 4) as u64;
    let params = streaming(-1.0, -14.0, Leveling::medium());
    let prepared = prepare(|| Ok(&pcm[..]), fmt, frames, &params, &mut |_, _, _| true).unwrap();
    let mut whole = Vec::new();
    Maximizer::new(&pcm[..], &prepared)
        .read_to_end(&mut whole)
        .unwrap();
    let mut pieces = Vec::new();
    let mut m = Maximizer::new(&pcm[..], &prepared);
    let mut buf = [0u8; 4093];
    loop {
        let n = m.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        pieces.extend_from_slice(&buf[..n]);
    }
    assert!(whole == pieces);
}

#[test]
fn a_measurement_hands_over_its_steps_as_it_goes() {
    let (fmt, pcm) = load();
    let frames = (pcm.len() / 4) as u64;
    let (mut k, mut peaks, mut batches) = (Vec::new(), Vec::new(), 0);
    let p = measure_with(&pcm[..], fmt, frames, &mut |_, _, _| true, &mut |ks, ps| {
        batches += 1;
        k.extend_from_slice(ks);
        peaks.extend_from_slice(ps);
    })
    .unwrap();
    assert!(
        batches > 3,
        "steps should arrive during the pass, not once at the end ({batches})"
    );
    assert!(k.len() >= p.hops() - 1 && k == p.k_ms[..k.len()] && peaks == p.peak[..k.len()]);
    // The same numbers as the plain measurement.
    let plain = measure(&pcm[..], fmt, frames, &mut |_, _, _| true).unwrap();
    assert_eq!(plain.k_ms, p.k_ms);
}
