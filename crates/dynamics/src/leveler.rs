//! The leveller: turns a loudness profile into a slowly-moving gain curve.
//!
//! Because the whole programme has been measured before anything is changed, the curve can start
//! moving *before* a loud stretch arrives (lookahead is free offline) and never has to guess.
//!
//! What it does, in short:
//! 1. One static gain brings the whole set's integrated loudness to the target.
//! 2. Stretches that are louder than their surroundings (a beatmix where two tracks add up) are
//!    turned down, and stretches that are quieter (a track whose gain was left low) are turned up,
//!    but only by a fraction (`strength`) of the difference, only beyond a small dead zone, and never
//!    by more than a limit. Very deep quiet passages (breakdowns, intros) are left alone, so the
//!    energy arc of the set survives.
//! 3. The result is smoothed over seconds, with the dips starting early, so nothing pumps.
//!
//! "Surroundings" is a local average over several minutes, so a set that builds up slowly keeps
//! its slow build (see `arc_flatten`).

use crate::loudness::{lufs, Profile};

const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// Most the slow build-up of a set is ever evened out by.
const ARC_LIMIT_DB: f64 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Leveling {
    /// Fraction (0..1) of a stretch's difference from its surroundings that is corrected.
    pub strength: f64,
    /// Largest turn-up, in dB.
    pub max_boost_db: f64,
    /// Largest turn-down, in dB.
    pub max_cut_db: f64,
    /// Differences smaller than this (in LU) are left alone: normal groove.
    pub dead_zone_lu: f64,
    /// A stretch this many LU below its surroundings is no longer boosted at all (fully gone by `.1`).
    pub breakdown_lu: (f64, f64),
    /// Seconds ahead of a loud stretch that the turn-down starts.
    pub lookahead_s: f64,
    /// Seconds the turn-down is held after it, before easing back.
    pub release_s: f64,
    /// How far (0..1) the slow build-up of the set is flattened toward its average. 0 keeps it.
    pub arc_flatten: f64,
    /// Length of the "surroundings" average, in seconds.
    pub reference_s: f64,
}

impl Leveling {
    /// No leveling: one gain for the whole set.
    pub const OFF: Leveling = Leveling {
        strength: 0.0,
        max_boost_db: 0.0,
        max_cut_db: 0.0,
        dead_zone_lu: 1.5,
        breakdown_lu: (8.0, 14.0),
        lookahead_s: 2.0,
        release_s: 4.0,
        arc_flatten: 0.0,
        reference_s: 360.0,
    };

    pub fn gentle() -> Leveling {
        Leveling {
            strength: 0.4,
            max_boost_db: 3.0,
            max_cut_db: 3.0,
            ..Leveling::OFF
        }
    }

    pub fn medium() -> Leveling {
        Leveling {
            strength: 0.6,
            max_boost_db: 5.0,
            max_cut_db: 5.0,
            arc_flatten: 0.2,
            ..Leveling::OFF
        }
    }

    pub fn strong() -> Leveling {
        Leveling {
            strength: 0.85,
            max_boost_db: 8.0,
            max_cut_db: 8.0,
            arc_flatten: 0.5,
            ..Leveling::OFF
        }
    }

    pub fn is_off(&self) -> bool {
        self.strength <= 0.0 && self.arc_flatten <= 0.0
    }
}

/// Gain to apply, sampled once per profile step (at the middle of each step).
#[derive(Debug, Clone)]
pub struct GainCurve {
    pub hop_frames: usize,
    /// Linear gain at the middle of each step.
    pub gain: Vec<f32>,
    /// The single static part, in dB (for display).
    pub static_db: f64,
}

impl GainCurve {
    pub fn unity(hop_frames: usize) -> GainCurve {
        GainCurve {
            hop_frames,
            gain: vec![1.0],
            static_db: 0.0,
        }
    }

    /// Fill `out` with the gain for frames `first..first + out.len()`, interpolating between steps.
    pub fn fill(&self, first: u64, out: &mut [f32]) {
        let hop = self.hop_frames as f64;
        let last = self.gain.len() - 1;
        let mut pos = (first as f64 + 0.5) / hop - 0.5;
        let step = 1.0 / hop;
        for o in out.iter_mut() {
            *o = if pos <= 0.0 {
                self.gain[0]
            } else if pos >= last as f64 {
                self.gain[last]
            } else {
                let i = pos as usize;
                let t = (pos - i as f64) as f32;
                self.gain[i] + (self.gain[i + 1] - self.gain[i]) * t
            };
            pos += step;
        }
    }

    /// The gain at one frame, in dB.
    pub fn db_at(&self, frame: u64) -> f32 {
        let mut g = [1.0f32];
        self.fill(frame, &mut g);
        20.0 * g[0].log10()
    }

    pub fn max_db(&self) -> f64 {
        20.0 * f64::from(self.gain.iter().copied().fold(f32::MIN_POSITIVE, f32::max)).log10()
    }

    pub fn min_db(&self) -> f64 {
        20.0 * f64::from(self.gain.iter().copied().fold(f32::MAX, f32::min)).log10()
    }
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Plan the gain so the programme lands on `target_lufs` (before the limiter).
pub fn plan(profile: &Profile, target_lufs: f64, lv: &Leveling) -> GainCurve {
    let integrated = profile.integrated();
    let n = profile.hops();
    if !integrated.is_finite() || n == 0 {
        return GainCurve::unity(profile.hop_frames);
    }
    let static_db = target_lufs - integrated;
    let hop = profile.hop_secs();

    // Short-term loudness per step, and which steps are music (above the gate and not a long silence).
    let st: Vec<f64> = (0..n).map(|i| profile.short_term(i)).collect();
    let audible: Vec<bool> = st
        .iter()
        .map(|&l| l > ABSOLUTE_GATE_LUFS.max(integrated - 20.0))
        .collect();

    // Local average of loudness (in dB) over the audible steps within +-reference_s/2.
    let mut pre_db = vec![0.0; n + 1];
    let mut pre_n = vec![0usize; n + 1];
    for i in 0..n {
        pre_db[i + 1] = pre_db[i] + if audible[i] { st[i] } else { 0.0 };
        pre_n[i + 1] = pre_n[i] + usize::from(audible[i]);
    }
    let half = ((lv.reference_s / 2.0 / hop) as usize).max(1);
    let reference = |i: usize| -> f64 {
        let (a, b) = (i.saturating_sub(half), (i + half).min(n));
        let count = pre_n[b] - pre_n[a];
        if count == 0 {
            integrated
        } else {
            (pre_db[b] - pre_db[a]) / count as f64
        }
    };

    // Wanted correction per step, in dB relative to the static gain.
    let mut want = vec![0.0f64; n];
    for i in 0..n {
        if !audible[i] {
            continue; // leave silence and near-silence at the static gain
        }
        let local = reference(i);
        let dev = st[i] - local;
        let d = if dev > 0.0 {
            -(lv.strength * (dev - lv.dead_zone_lu).max(0.0)).min(lv.max_cut_db)
        } else {
            let deficit = -dev;
            let fade = 1.0 - smoothstep(lv.breakdown_lu.0, lv.breakdown_lu.1, deficit);
            (lv.strength * (deficit - lv.dead_zone_lu).max(0.0)).min(lv.max_boost_db) * fade
        };
        want[i] = d + (-lv.arc_flatten * (local - integrated)).clamp(-ARC_LIMIT_DB, ARC_LIMIT_DB);
    }

    // Lookahead and release: take the lowest wanted value from `lookahead` ahead to `release` behind
    // (so a turn-down starts early and is held), then smooth with a window no wider than either side,
    // which keeps the smoothed curve at or below the wanted one wherever a cut is needed.
    let ahead = (lv.lookahead_s / hop).round() as usize;
    let behind = (lv.release_s / hop).round().max(ahead as f64) as usize;
    let mut low = vec![0.0f64; n];
    for (i, slot) in low.iter_mut().enumerate() {
        let (a, b) = (i.saturating_sub(behind), (i + ahead + 1).min(n));
        // Boosts survive only where they last for the whole window.
        *slot = want[a..b].iter().copied().fold(f64::INFINITY, f64::min);
    }
    let w = ahead.max(1);
    let kernel: Vec<f64> = (0..=2 * w)
        .map(|k| {
            0.5 - 0.5 * (2.0 * std::f64::consts::PI * (k as f64 + 1.0) / (2 * w + 2) as f64).cos()
        })
        .collect();
    let ksum: f64 = kernel.iter().sum();
    let gain: Vec<f32> = (0..n)
        .map(|i| {
            let mut acc = 0.0;
            for (k, kv) in kernel.iter().enumerate() {
                // Edges repeat the nearest step.
                let j = (i + k).saturating_sub(w).min(n - 1);
                acc += kv * low[j];
            }
            let db = static_db + acc / ksum;
            10f64.powf(db / 20.0) as f32
        })
        .collect();

    GainCurve {
        hop_frames: profile.hop_frames,
        gain,
        static_db,
    }
}

/// Peak-only mode: the single gain that puts the highest true peak on the ceiling.
pub fn normalize_peak(profile: &Profile, ceiling_db: f64) -> GainCurve {
    let peak = f64::from(profile.peak_max());
    if peak <= 0.0 {
        return GainCurve::unity(profile.hop_frames);
    }
    let db = ceiling_db - 20.0 * peak.log10();
    GainCurve {
        hop_frames: profile.hop_frames,
        gain: vec![10f64.powf(db / 20.0) as f32],
        static_db: db,
    }
}

/// Loudness of a step, for callers that want to show the curve next to it.
pub fn step_lufs(profile: &Profile, i: usize) -> f64 {
    lufs(profile.k_ms[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loudness::Analyzer;
    use std::f64::consts::PI;

    const RATE: f64 = 22_050.0;

    /// A stereo tone (997 Hz) whose level in dBFS follows `level(t)`.
    fn programme(secs: f64, level: impl Fn(f64) -> f64) -> Profile {
        let mut a = Analyzer::new(RATE, 2, false);
        let n = (RATE * secs) as usize;
        let mut chunk = Vec::with_capacity(8192);
        for i in 0..n {
            let t = i as f64 / RATE;
            let v = (10f64.powf(level(t) / 20.0) * (2.0 * PI * 997.0 * t).sin()) as f32;
            chunk.push(v);
            chunk.push(v);
            if chunk.len() == 8192 {
                a.push(&chunk);
                chunk.clear();
            }
        }
        a.push(&chunk);
        a.finish()
    }

    /// Loudness the programme would have after the curve is applied: level + gain, per step.
    fn after(profile: &Profile, curve: &GainCurve, t: f64) -> f64 {
        let i = ((t / profile.hop_secs()) as usize).min(profile.hops() - 1);
        profile.short_term(i) + 20.0 * f64::from(curve.gain[i.min(curve.gain.len() - 1)]).log10()
    }

    #[test]
    fn a_steady_programme_only_gets_the_static_gain() {
        let p = programme(120.0, |_| -22.0);
        let c = plan(&p, -14.0, &Leveling::medium());
        assert!((c.static_db - 8.0).abs() < 0.1);
        assert!(
            c.max_db() - c.min_db() < 0.05,
            "{} {}",
            c.min_db(),
            c.max_db()
        );
    }

    #[test]
    fn leveling_off_is_one_flat_gain() {
        let p = programme(120.0, |t| {
            if (40.0..60.0).contains(&t) {
                -14.0
            } else {
                -22.0
            }
        });
        let c = plan(&p, -16.0, &Leveling::OFF);
        assert!(c.max_db() - c.min_db() < 1e-3);
    }

    #[test]
    fn a_loud_stretch_is_pulled_toward_the_rest_and_the_dip_starts_early() {
        // 4 minutes at -24, with 30 s at -18 (a beatmix adding up) in the middle.
        let p = programme(240.0, |t| {
            if (105.0..135.0).contains(&t) {
                -18.0
            } else {
                -24.0
            }
        });
        let lv = Leveling::medium();
        let c = plan(&p, -14.0, &lv);
        let flat = plan(&p, -14.0, &Leveling::OFF);
        let spread = |c: &GainCurve| after(&p, c, 120.0) - after(&p, c, 60.0);
        assert!(spread(&flat) > 5.5);
        assert!(
            spread(&c) < spread(&flat) - 2.0,
            "spread {} vs {}",
            spread(&c),
            spread(&flat)
        );
        // Gain is already falling before the loud stretch begins.
        let g = |t: f64| 20.0 * f64::from(c.gain[(t / p.hop_secs()) as usize]).log10();
        assert!(g(104.0) < g(60.0) - 0.5, "{} vs {}", g(104.0), g(60.0));
        // Never beyond the limit.
        assert!(c.static_db - c.min_db() <= lv.max_cut_db + 1e-6);
    }

    #[test]
    fn a_track_left_too_quiet_is_brought_up() {
        // 6 minutes: a normal track, one that is 5 dB low, a normal track.
        let p = programme(360.0, |t| {
            if (150.0..210.0).contains(&t) {
                -25.0
            } else {
                -20.0
            }
        });
        let flat = plan(&p, -14.0, &Leveling::OFF);
        let gap = |c: &GainCurve| after(&p, c, 100.0) - after(&p, c, 180.0);
        assert!(gap(&flat) > 4.5);
        let medium = plan(&p, -14.0, &Leveling::medium());
        let strong = plan(&p, -14.0, &Leveling::strong());
        assert!(
            gap(&medium) < gap(&flat) - 1.2,
            "{} vs {}",
            gap(&medium),
            gap(&flat)
        );
        assert!(
            gap(&strong) < gap(&medium) - 0.5,
            "{} vs {}",
            gap(&strong),
            gap(&medium)
        );
    }

    #[test]
    fn a_breakdown_stays_a_breakdown() {
        // -20 with a 30 s hole at -40: it must not be lifted toward the rest.
        let p = programme(240.0, |t| {
            if (100.0..130.0).contains(&t) {
                -40.0
            } else {
                -20.0
            }
        });
        let c = plan(&p, -14.0, &Leveling::strong());
        let hole = after(&p, &c, 115.0);
        let rest = after(&p, &c, 50.0);
        assert!(rest - hole > 17.0, "breakdown was lifted: {hole} vs {rest}");
    }

    #[test]
    fn silence_is_not_boosted_and_does_not_break_planning() {
        let p = programme(120.0, |t| if t < 60.0 { -60.0 } else { -20.0 });
        let c = plan(&p, -14.0, &Leveling::strong());
        // Boost is bounded: the static gain plus the limit.
        assert!(c.max_db() <= c.static_db + ARC_LIMIT_DB + Leveling::strong().max_boost_db + 1e-6);
        let silent = programme(30.0, |_| -200.0);
        assert!(plan(&silent, -14.0, &Leveling::medium())
            .gain
            .iter()
            .all(|&g| g == 1.0));
    }

    #[test]
    fn the_curve_changes_slowly() {
        let p = programme(240.0, |t| {
            if (105.0..135.0).contains(&t) {
                -16.0
            } else {
                -24.0
            }
        });
        let c = plan(&p, -14.0, &Leveling::strong());
        let db: Vec<f64> = c
            .gain
            .iter()
            .map(|&g| 20.0 * f64::from(g).log10())
            .collect();
        let fastest = db
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f64::max)
            / p.hop_secs();
        assert!(fastest < 4.0, "{fastest} dB/s");
    }

    #[test]
    fn interpolation_is_continuous_and_holds_at_the_ends() {
        let c = GainCurve {
            hop_frames: 100,
            gain: vec![1.0, 2.0, 1.0],
            static_db: 0.0,
        };
        let mut out = vec![0.0; 300];
        c.fill(0, &mut out);
        assert_eq!(out[0], 1.0);
        assert!((out[99] - 1.5).abs() < 0.02 || out[99] > 1.0);
        assert!(out.windows(2).all(|w| (w[1] - w[0]).abs() < 0.02));
        assert_eq!(out[299], 1.0);
        // Chunked fills give the same numbers as one big one.
        let mut two = vec![0.0; 300];
        c.fill(0, &mut two[..120]);
        c.fill(120, &mut two[120..]);
        assert_eq!(out, two);
    }

    #[test]
    fn peak_mode_puts_the_loudest_peak_on_the_ceiling() {
        let p = programme(10.0, |_| -12.0);
        let c = normalize_peak(&p, -0.1);
        assert!((c.static_db - 11.9).abs() < 0.3, "{}", c.static_db);
    }
}
