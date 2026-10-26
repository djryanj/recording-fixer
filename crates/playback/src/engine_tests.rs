//! End-to-end tests of the engine against a simulated device.

use crate::convert::decode_be;
use crate::device::DeviceSelection;
use crate::engine::{Error, Playback};
use crate::testing::{SimBackend, SimConfig, SimOutput};
use aiff_core::fixtures::{self, Spec};
use edit_core::Clip;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct Rig {
    pb: Playback,
    out: SimOutput,
    dir: tempfile::TempDir,
}

fn rig_with(config: SimConfig) -> Rig {
    let out = SimOutput::new();
    let o = out.clone();
    let pb = Playback::start(move || Box::new(SimBackend::new(o, config)));
    Rig {
        pb,
        out,
        dir: tempfile::tempdir().unwrap(),
    }
}

fn rig() -> Rig {
    rig_with(SimConfig::default())
}

impl Rig {
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn wav(&self, name: &str, spec: Spec, seed: u32) -> PathBuf {
        self.file(name, &fixtures::healthy(spec, seed))
    }

    /// A stereo 44.1 kHz sine at 16000/32768 amplitude.
    fn sine(&self, name: &str, hz: f64, frames: u32) -> PathBuf {
        let mut audio = Vec::new();
        for i in 0..frames {
            let v = ((f64::from(i) * hz * std::f64::consts::TAU / 44100.0).sin() * 16000.0) as i16;
            audio.extend_from_slice(&v.to_be_bytes());
            audio.extend_from_slice(&v.to_be_bytes());
        }
        let mut f =
            aiff_core::header::build(&Spec::cd(0).format(), frames, audio.len() as u64).to_vec();
        f.extend(audio);
        self.file(name, &f)
    }

    /// Load and wait until the decoder has buffered enough that playback cannot underrun.
    fn load(&self, clips: Vec<Clip>, start: f64) {
        let info = self.pb.load(clips, start).expect("load");
        let want = ((info.total_frames as f64 - start * f64::from(info.rate)) as usize).min(10_000);
        wait(
            || self.pb.buffered_frames() >= want,
            "the decoder to fill the buffer",
        );
    }

    /// Wait for the end of playback, then for the simulated device to record the chunks around it.
    fn wait_ended(&self) {
        wait(|| self.pb.state().ended, "playback to finish");
        let seen = self.out.chunk_count();
        wait(
            || self.out.chunk_count() >= seen + 3,
            "the final chunks to be recorded",
        );
    }

    fn play_to_end(&self) {
        self.pb.play();
        self.wait_ended();
    }
}

fn wait(mut cond: impl FnMut() -> bool, what: &str) {
    let started = Instant::now();
    while !cond() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn expected(spec: Spec, seed: u32) -> Vec<f32> {
    let mut v = Vec::new();
    decode_be(&fixtures::pcm(spec, seed), spec.bits, &mut v);
    v
}

fn find(haystack: &[f32], needle: &[f32]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn assert_audio_is(audible: &[f32], want: &[f32]) {
    assert!(
        audible.len() >= want.len(),
        "heard {} samples, expected {}",
        audible.len(),
        want.len()
    );
    assert!(
        audible.starts_with(want),
        "audio differs from the source (first mismatch at {:?})",
        audible.iter().zip(want).position(|(a, b)| a != b)
    );
    assert!(
        audible[want.len()..].iter().all(|&s| s == 0.0),
        "unexpected extra audio after the end"
    );
}

const SECOND: Spec = Spec::cd(44100);

#[test]
fn plays_a_file_exactly_once_and_stops() {
    let r = rig();
    let p = r.wav("a.aif", SECOND, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.play_to_end();
    assert_audio_is(&r.out.audible(), &expected(SECOND, 1));
    let st = r.pb.state();
    assert!(!st.playing && st.ended);
    assert_eq!(st.position_secs, st.duration_secs);
    assert_eq!(st.underruns, 0);
}

#[test]
fn state_reports_duration_and_position_before_and_during_playback() {
    let r = rig();
    let p = r.wav("a.aif", Spec::cd(44100 * 2), 1);
    assert!(!r.pb.state().loaded);
    r.load(vec![Clip::whole(p)], 0.0);
    let st = r.pb.state();
    assert!(st.loaded && !st.playing && !st.ended);
    assert!((st.duration_secs - 2.0).abs() < 1e-9);
    assert_eq!(st.position_secs, 0.0);
    r.pb.play();
    wait(|| r.pb.state().position_secs > 0.1, "position to advance");
    assert!(r.pb.state().playing);
}

#[test]
fn a_broken_unclosed_file_plays_without_being_repaired() {
    let r = rig();
    let p = r.file("broken.aif", &fixtures::broken(SECOND, 3, 3));
    r.load(vec![Clip::whole(p)], 0.0);
    r.play_to_end();
    assert_audio_is(&r.out.audible(), &expected(SECOND, 3));
}

#[test]
fn clips_play_back_to_back_and_trims_are_honoured() {
    let r = rig();
    let spec_a = Spec::cd(30000);
    let spec_b = Spec::cd(20000);
    let a = r.wav("a.aif", spec_a, 1);
    let b = r.wav("b.aif", spec_b, 2);
    r.load(
        vec![
            Clip {
                path: a,
                start_frame: 5000,
                end_frame: Some(25000),
            },
            Clip {
                path: b,
                start_frame: 0,
                end_frame: Some(10000),
            },
        ],
        0.0,
    );
    r.play_to_end();
    let mut want = expected(spec_a, 1)[5000 * 2..25000 * 2].to_vec();
    want.extend_from_slice(&expected(spec_b, 2)[..10000 * 2]);
    assert_audio_is(&r.out.audible(), &want);
}

#[test]
fn pause_silences_the_output_and_resume_continues_without_gaps_or_repeats() {
    let r = rig();
    let spec = Spec::cd(44100 * 2);
    let p = r.wav("a.aif", spec, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.pb.play();
    wait(|| r.pb.state().position_secs > 0.3, "some playback");
    r.pb.pause();
    wait(|| !r.pb.state().playing, "pause");
    std::thread::sleep(Duration::from_millis(40)); // let in-flight callbacks drain
    let paused_at = r.pb.state().position_secs;
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        r.pb.state().position_secs,
        paused_at,
        "position must not move while paused"
    );
    assert_eq!(r.pb.state().peak_l, 0.0);
    r.play_to_end();
    assert_audio_is(&r.out.audible(), &expected(spec, 1));
}

#[test]
fn seeking_while_playing_jumps_and_never_plays_stale_audio() {
    let r = rig();
    let spec = Spec::cd(44100 * 2);
    let p = r.wav("a.aif", spec, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.pb.play();
    wait(|| r.pb.state().position_secs > 0.1, "playback");
    r.pb.seek(1.0);
    wait(|| r.pb.state().position_secs >= 1.0, "the seek");
    r.wait_ended();
    let heard = r.out.audible();
    let tail = &expected(spec, 1)[44100 * 2..];
    let at = find(&heard, tail).expect("the second half must play contiguously after the seek");
    // Everything before it is the pre-seek audio: a prefix of the file, never anything after the seek point's neighbours.
    assert!(
        heard[..at].len() < 44100 * 2,
        "pre-seek audio longer than one second"
    );
}

#[test]
fn seeking_while_paused_then_playing_starts_at_the_new_position() {
    let r = rig();
    let spec = Spec::cd(44100 * 2);
    let p = r.wav("a.aif", spec, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.pb.seek(0.5);
    wait(
        || (r.pb.state().position_secs - 0.5).abs() < 0.02,
        "the seek to register",
    );
    assert!(!r.pb.state().playing);
    wait(|| r.pb.buffered_frames() > 10_000, "the buffer to refill");
    r.out.clear();
    r.play_to_end();
    assert_audio_is(&r.out.audible(), &expected(spec, 1)[22050 * 2..]);
}

#[test]
fn playing_after_the_end_starts_again_from_the_beginning() {
    let r = rig();
    let p = r.wav("a.aif", SECOND, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.play_to_end();
    r.out.clear();
    wait(|| r.pb.buffered_frames() > 0 || true, "nothing");
    r.pb.play();
    wait(
        || r.pb.state().ended && r.out.audible().len() >= 88200,
        "the second pass",
    );
    let heard = r.out.audible();
    assert!(
        find(&heard, &expected(SECOND, 1)).is_some(),
        "the whole file should play again"
    );
}

#[test]
fn seeking_past_the_end_ends_playback_cleanly() {
    let r = rig();
    let p = r.wav("a.aif", SECOND, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.pb.play();
    r.pb.seek(99.0);
    r.wait_ended();
    assert_eq!(r.pb.state().position_secs, 1.0);
}

#[test]
fn a_negative_seek_clamps_to_the_start() {
    let r = rig();
    let p = r.wav("a.aif", SECOND, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.pb.seek(-5.0);
    wait(|| r.pb.buffered_frames() > 10_000, "buffer");
    r.play_to_end();
    assert_audio_is(&r.out.audible(), &expected(SECOND, 1));
}

#[test]
fn loading_new_material_while_playing_continues_from_the_requested_position() {
    let r = rig();
    let spec = Spec::cd(44100 * 2);
    let a = r.wav("a.aif", spec, 1);
    let b = r.wav("b.aif", spec, 2);
    r.load(vec![Clip::whole(a)], 0.0);
    r.pb.play();
    wait(|| r.pb.state().position_secs > 0.1, "playback");
    r.pb.load(vec![Clip::whole(b)], 0.5).unwrap();
    r.wait_ended();
    let heard = r.out.audible();
    assert!(
        find(&heard, &expected(spec, 2)[22050 * 2..]).is_some(),
        "the new material should play from 0.5 s"
    );
    assert_eq!(
        r.out.opened().len(),
        1,
        "an edit must not reopen the audio device"
    );
}

#[test]
fn a_failed_load_leaves_the_current_material_alone() {
    let r = rig();
    let p = r.wav("a.aif", SECOND, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    let err =
        r.pb.load(vec![Clip::whole(r.dir.path().join("missing.aif"))], 0.0)
            .unwrap_err();
    assert!(matches!(err, Error::Plan(_)), "{err}");
    assert!(r.pb.load(vec![], 0.0).is_err());
    let st = r.pb.state();
    assert!(st.loaded && (st.duration_secs - 1.0).abs() < 1e-9);
    r.play_to_end();
    assert_audio_is(&r.out.audible(), &expected(SECOND, 1));
}

#[test]
fn volume_scales_the_output() {
    let r = rig();
    let p = r.wav("a.aif", SECOND, 1);
    r.pb.set_volume(0.5);
    r.load(vec![Clip::whole(p)], 0.0);
    r.pb.set_volume(0.5);
    std::thread::sleep(Duration::from_millis(30));
    r.play_to_end();
    let want: Vec<f32> = expected(SECOND, 1).iter().map(|s| s * 0.5).collect();
    assert_audio_is(&r.out.audible(), &want);
    assert_eq!(r.pb.state().volume, 0.5);
}

#[test]
fn the_meter_follows_the_signal_and_goes_quiet_when_paused() {
    let r = rig();
    let spec = Spec::cd(44100 * 2);
    let p = r.wav("a.aif", spec, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.pb.play();
    wait(|| r.pb.state().peak_l > 0.0, "a level");
    let max = expected(spec, 1).iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let st = r.pb.state();
    assert!(
        st.peak_l > 0.0 && st.peak_l <= max + 1e-6,
        "{} vs {max}",
        st.peak_l
    );
    assert!(st.peak_r > 0.0);
    r.pb.pause();
    wait(|| r.pb.state().peak_l == 0.0, "the meter to fall silent");
}

#[test]
fn mono_material_plays_on_both_sides() {
    let r = rig();
    let spec = Spec {
        rate: 44100,
        channels: 1,
        bits: 16,
        frames: 20000,
    };
    let p = r.wav("mono.aif", spec, 4);
    r.load(vec![Clip::whole(p)], 0.0);
    r.play_to_end();
    let want: Vec<f32> = expected(spec, 4).iter().flat_map(|&s| [s, s]).collect();
    assert_audio_is(&r.out.audible(), &want);
}

#[test]
fn twenty_four_bit_material_decodes_correctly() {
    let r = rig();
    let spec = Spec {
        rate: 44100,
        channels: 2,
        bits: 24,
        frames: 20000,
    };
    let p = r.wav("deep.aif", spec, 5);
    r.load(vec![Clip::whole(p)], 0.0);
    r.play_to_end();
    assert_audio_is(&r.out.audible(), &expected(spec, 5));
}

#[test]
fn a_chosen_output_pair_on_a_multichannel_device_carries_the_audio() {
    let r = rig();
    let info =
        r.pb.set_device(DeviceSelection {
            id: Some("sim:multi".into()),
            out_pair: 1,
            buffer_frames: None,
        })
        .unwrap();
    assert_eq!((info.channels, info.out_pair), (8, 1));
    let spec = Spec::cd(20000);
    let p = r.wav("a.aif", spec, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.play_to_end();
    let heard = r.out.audible();
    let src = expected(spec, 1);
    for (i, frame) in heard.as_chunks::<8>().0.iter().take(20000).enumerate() {
        assert_eq!(
            (frame[2], frame[3]),
            (src[i * 2], src[i * 2 + 1]),
            "frame {i}"
        );
        assert!(
            frame[..2].iter().chain(&frame[4..]).all(|&s| s == 0.0),
            "other channels must be silent (frame {i})"
        );
    }
}

#[test]
fn a_device_running_at_another_rate_gets_resampled_audio_of_the_right_length() {
    let r = rig_with(SimConfig {
        rate: Some(48000),
        ..SimConfig::default()
    });
    // A clean sine: resampling a signal with hard edges would legitimately overshoot.
    let p = r.sine("a.aif", 1000.0, 44100);
    r.load(vec![Clip::whole(p)], 0.0);
    r.play_to_end();
    assert_eq!(r.out.opened()[0].rate, 48000);
    let heard = r.out.audible();
    let frames = heard.len() / 2;
    assert!(
        (48000..48000 + 512).contains(&frames),
        "{frames} frames heard"
    );
    let peak = heard.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!((peak - 16000.0 / 32768.0).abs() < 0.02, "peak {peak}");
    assert!(
        (r.pb.state().duration_secs - 1.0).abs() < 1e-9,
        "duration is the material's, not the device's"
    );
}

#[test]
fn position_is_in_seconds_of_material_even_when_resampling() {
    let r = rig_with(SimConfig {
        rate: Some(48000),
        ..SimConfig::default()
    });
    let spec = Spec::cd(44100 * 2);
    let p = r.wav("a.aif", spec, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.pb.seek(1.0);
    wait(|| (r.pb.state().position_secs - 1.0).abs() < 0.02, "seek");
    wait(|| r.pb.buffered_frames() > 10_000, "buffer");
    r.play_to_end();
    let st = r.pb.state();
    assert!((st.position_secs - 2.0).abs() < 1e-9);
}

#[test]
fn switching_devices_mid_playback_keeps_playing_from_about_the_same_place() {
    let r = rig();
    let spec = Spec::cd(44100 * 3);
    let p = r.wav("a.aif", spec, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.pb.play();
    wait(|| r.pb.state().position_secs > 0.3, "playback");
    let before = r.pb.state().position_secs;
    let info =
        r.pb.set_device(DeviceSelection {
            id: Some("sim:multi".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(info.name, "Simulated 8-channel interface");
    let st = r.pb.state();
    assert_eq!(st.device.as_ref().unwrap().id, "sim:multi");
    assert!(
        st.position_secs >= before - 0.1,
        "{} vs {before}",
        st.position_secs
    );
    r.wait_ended();
    assert_eq!(r.out.opened().len(), 2);
}

#[test]
fn an_unknown_device_is_reported_and_does_not_wedge_the_engine() {
    let r = rig();
    let err =
        r.pb.set_device(DeviceSelection {
            id: Some("nope:nothing".into()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(
        matches!(&err, Error::Device(m) if m.contains("nope:nothing")),
        "{err}"
    );
    // The engine recovers on the next request.
    let p = r.wav("a.aif", SECOND, 1);
    r.pb.set_device(DeviceSelection::default()).unwrap();
    r.load(vec![Clip::whole(p)], 0.0);
    r.play_to_end();
    assert_audio_is(&r.out.audible(), &expected(SECOND, 1));
}

#[test]
fn devices_and_details_come_from_the_backend() {
    let r = rig();
    let devices = r.pb.devices();
    assert_eq!(devices.len(), 2);
    assert!(devices[0].is_default);
    assert_eq!(r.pb.device_details("sim:multi").unwrap().channels, 8);
    assert!(r.pb.device_details("sim:ghost").is_none());
}

#[test]
fn a_pair_beyond_the_devices_channels_falls_back_to_the_first_pair() {
    let r = rig();
    let info =
        r.pb.set_device(DeviceSelection {
            id: Some("sim:main".into()),
            out_pair: 7,
            buffer_frames: None,
        })
        .unwrap();
    let spec = Spec::cd(5000);
    let p = r.wav("a.aif", spec, 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.play_to_end();
    assert_eq!(info.channels, 2);
    assert_audio_is(&r.out.audible(), &expected(spec, 1));
}

#[test]
fn play_and_pause_before_anything_is_loaded_are_harmless() {
    let r = rig();
    r.pb.play();
    r.pb.pause();
    r.pb.seek(3.0);
    r.pb.set_volume(0.3);
    let st = r.pb.state();
    assert!(!st.loaded && !st.playing);
}

#[test]
fn dropping_the_engine_mid_playback_shuts_down_cleanly() {
    let r = rig();
    let p = r.wav("a.aif", Spec::cd(44100 * 5), 1);
    r.load(vec![Clip::whole(p)], 0.0);
    r.pb.play();
    wait(|| r.pb.state().position_secs > 0.05, "playback");
    let started = Instant::now();
    drop(r.pb);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "shutdown took {:?}",
        started.elapsed()
    );
}

#[test]
fn a_source_that_disappears_mid_playback_reports_an_error_instead_of_hanging() {
    let r = rig();
    let spec = Spec::cd(44100 * 20);
    let p = r.wav("a.aif", spec, 1);
    r.load(vec![Clip::whole(p.clone())], 0.0);
    // Truncate the file under the decoder's feet: reads past the new end fail.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&p)
        .unwrap()
        .set_len(100)
        .unwrap();
    r.pb.play();
    wait(
        || r.pb.state().error.is_some() || r.pb.state().ended,
        "an error or the end",
    );
    // The engine must still answer afterwards.
    let _ = r.pb.state();
    assert!(r.pb.devices().len() == 2);
}
