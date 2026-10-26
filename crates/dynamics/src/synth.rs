//! A made-up DJ set made of kicks, bass notes, beeps and hats: no recording, so nothing to license.
//!
//! It is built to exercise the maximizer the way a real set does, in 46 seconds:
//!
//! | time    | what                                                                            |
//! |---------|---------------------------------------------------------------------------------|
//! | 0 - 8   | track A on its own, the reference level                                          |
//! | 8 - 16  | beatmix: A fades out while B fades in, kicks aligned, the two basslines adding   |
//! | 16 - 24 | track B alone, but 5 dB lower (a track whose gain was left too low)              |
//! | 24 - 30 | breakdown: pads and hats only, far quieter (must stay a breakdown)              |
//! | 30 - 38 | drop: track C, hotter, with sharp clicks on top (high peak-to-loudness ratio)    |
//! | 38 - 40 | silence                                                                          |
//! | 40 - 46 | track A again                                                                    |
//!
//! Everything is deterministic. `tests/fixtures/beeps-set.aif` is this rendered at 16 bits.

use std::f64::consts::PI;

pub const RATE: f64 = 22_050.0;
pub const SECONDS: f64 = 46.0;
const MASTER_DB: f64 = -6.7;
const BEAT: f64 = 60.0 / 128.0;

/// (name, start second, end second) of each stretch, for tests and for pointing at in the app.
pub const SECTIONS: [(&str, f64, f64); 7] = [
    ("A alone", 0.0, 8.0),
    ("beatmix A+B", 8.0, 16.0),
    ("B alone, 5 dB low", 16.0, 24.0),
    ("breakdown", 24.0, 30.0),
    ("drop with clicks", 30.0, 38.0),
    ("silence", 38.0, 40.0),
    ("A again", 40.0, 46.0),
];

struct Track {
    bass_hz: f64,
    lead: [f64; 4],
    pan: f64,
}

const TRACK_A: Track = Track {
    bass_hz: 55.0,
    lead: [440.0, 523.25, 659.25, 783.99],
    pan: 0.35,
};
const TRACK_B: Track = Track {
    bass_hz: 58.27,
    lead: [466.16, 554.37, 698.46, 880.0],
    pan: -0.35,
};
const TRACK_C: Track = Track {
    bass_hz: 49.0,
    lead: [392.0, 493.88, 587.33, 740.0],
    pan: 0.0,
};

struct Noise(u64);
impl Noise {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / (1u64 << 31) as f64 * 2.0 - 1.0
    }
}

fn kick(t: f64) -> f64 {
    if t > 0.35 {
        return 0.0;
    }
    let phase = 2.0 * PI * (45.0 * t + 75.0 * 0.04 * (1.0 - (-t / 0.04).exp()));
    phase.sin() * (-t / 0.11).exp()
}

/// Add one track's parts into `left`/`right` between `from` and `to` seconds, scaled by `level(t)`.
fn track(
    left: &mut [f32],
    right: &mut [f32],
    tr: &Track,
    (from, to): (f64, f64),
    level: &dyn Fn(f64) -> f64,
    parts: Parts,
) {
    let mut noise = Noise(0x1234_5678 ^ (tr.bass_hz.to_bits()));
    let (i0, i1) = (
        (from * RATE) as usize,
        ((to * RATE) as usize).min(left.len()),
    );
    let mut prev_noise = 0.0;
    for i in i0..i1 {
        let t = i as f64 / RATE;
        let in_beat = t.rem_euclid(BEAT);
        let in_eighth = t.rem_euclid(BEAT / 2.0);
        let step = ((t / (BEAT / 2.0)) as usize) % 8;
        let mut mono = 0.0;
        let mut side = 0.0;
        if parts.kick {
            mono += 0.8 * kick(in_beat);
        }
        if parts.bass {
            // Off-beat bass notes: loudest right after each kick, in the gaps between kicks.
            let env = (-in_eighth / 0.16).exp() * if step % 2 == 1 { 1.0 } else { 0.35 };
            mono += 0.5 * env * (2.0 * PI * tr.bass_hz * t).sin();
        }
        if parts.lead {
            let note = tr.lead[step % 4];
            let env = (-in_eighth / 0.09).exp();
            let v = env * ((2.0 * PI * note * t).sin() + 0.3 * (4.0 * PI * note * t).sin());
            mono += 0.09 * v;
            side += 0.07 * v * tr.pan.signum();
        }
        let n = noise.next();
        if parts.hats {
            let hp = n - prev_noise; // crude high-pass
            let off = (t + BEAT / 4.0).rem_euclid(BEAT / 2.0);
            mono += 0.07 * hp * (-off / 0.02).exp() * 0.5;
        }
        prev_noise = n;
        if parts.pad {
            let trem = 0.6 + 0.4 * (2.0 * PI * 0.25 * t).sin();
            let pad = [220.0, 277.18, 329.63]
                .iter()
                .map(|f| (2.0 * PI * f * t).sin() + (2.0 * PI * (f * 1.004) * t).sin())
                .sum::<f64>();
            mono += 0.04 * trem * pad;
        }
        if parts.clicks {
            let c = in_eighth;
            if c < 0.004 {
                mono += 0.9
                    * if step.is_multiple_of(2) { 1.0 } else { 0.6 }
                    * (1.0 - c / 0.004)
                    * n.signum();
            }
        }
        let g = level(t);
        left[i] += ((mono + side) * g) as f32;
        right[i] += ((mono - side) * g) as f32;
    }
}

#[derive(Clone, Copy)]
struct Parts {
    kick: bool,
    bass: bool,
    lead: bool,
    hats: bool,
    pad: bool,
    clicks: bool,
}

const FULL: Parts = Parts {
    kick: true,
    bass: true,
    lead: true,
    hats: true,
    pad: false,
    clicks: false,
};

fn db(x: f64) -> f64 {
    10f64.powf(x / 20.0)
}

fn ramp(t: f64, a: f64, b: f64) -> f64 {
    ((t - a) / (b - a)).clamp(0.0, 1.0)
}

/// The set, as interleaved stereo f32 at `RATE`.
pub fn beeps_set() -> Vec<f32> {
    let n = (SECONDS * RATE) as usize;
    let (mut l, mut r) = (vec![0f32; n], vec![0f32; n]);
    let g = db(-1.0);
    track(
        &mut l,
        &mut r,
        &TRACK_A,
        (0.0, 16.0),
        &|t| g * (1.0 - ramp(t, 11.0, 16.0)),
        FULL,
    );
    track(
        &mut l,
        &mut r,
        &TRACK_B,
        (8.0, 24.0),
        &|t| g * ramp(t, 8.0, 10.0) * if t >= 16.0 { db(-5.0) } else { 1.0 },
        FULL,
    );
    track(
        &mut l,
        &mut r,
        &TRACK_B,
        (24.0, 30.0),
        &|_| db(3.0),
        Parts {
            kick: false,
            bass: false,
            lead: false,
            hats: true,
            pad: true,
            clicks: false,
        },
    );
    track(
        &mut l,
        &mut r,
        &TRACK_C,
        (30.0, 38.0),
        &|_| db(1.0),
        Parts {
            clicks: true,
            ..FULL
        },
    );
    track(&mut l, &mut r, &TRACK_A, (40.0, 46.0), &|_| g, FULL);
    // Level of a typical recorded set: about -20 LUFS with peaks a few dB under full scale.
    let master = db(MASTER_DB) as f32;
    for v in l.iter_mut().chain(r.iter_mut()) {
        *v *= master;
    }
    // Short fades at the cuts so the set has no clicks of its own.
    let edge = (0.01 * RATE) as usize;
    for (a, b) in [(24.0, 30.0), (30.0, 38.0), (40.0, 46.0)] {
        for k in 0..edge {
            let f = k as f32 / edge as f32;
            let (s, e) = ((a * RATE) as usize + k, (b * RATE) as usize - 1 - k);
            l[s] *= f;
            r[s] *= f;
            l[e] *= f;
            r[e] *= f;
        }
    }
    l.iter().zip(&r).flat_map(|(&a, &b)| [a, b]).collect()
}

/// 16-bit big-endian interleaved bytes, as stored in an AIFF.
pub fn to_pcm16(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|&x| ((x * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_be_bytes())
        .collect()
}
