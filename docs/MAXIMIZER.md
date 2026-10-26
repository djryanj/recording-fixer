# Loudness maximizer

Built into the export. It exists because a set recorded off Serato's output almost always needs dynamics work before it is shared: the level is low, beatmixes are louder than single tracks, and one track is sometimes quieter than the rest. A normal maximizer sets one gain for the whole file; this one follows the set.

It lives in `crates/dynamics` (pure sample maths, no Tauri) and runs in the export pipeline as `read → loudness → fades → encode`. It is **off** unless chosen, because it changes the audio.

## What it does

1. **Measures** the whole programme in one streaming read (ITU BS.1770 / EBU R128 K-weighted loudness in 100 ms steps; momentary, short-term, gated integrated, loudness range, true peak).
2. **Plans** a gain curve with unlimited lookahead (it has seen the whole set):
   - one static gain brings the integrated loudness to the target;
   - stretches louder than their surroundings (two tracks adding up) are turned down, and stretches quieter than their surroundings (a track left low) are turned up, by a *fraction* of the difference, beyond a dead zone, never past a limit;
   - very deep quiet passages (breakdowns, intros) are left alone, so the **energy arc survives**;
   - the "surroundings" are a local average over about six minutes, so a set that builds slowly keeps its build (`arc_flatten`, 0 for Gentle);
   - the turn-down starts two seconds before a loud stretch arrives and is held four seconds after, with a smooth (Hann) transition. Nothing pumps at beat rate: the shortest window is the 3 s short-term loudness.
3. **Rehearses** the whole chain once (up to twice) with the output discarded, to see how far the limiter pulled the loudness under the target, and corrects the gain. Lands within about 0.15 LU.
4. **Processes**: gain curve, then a **true-peak brickwall limiter**, then dither back to the input's bit depth.

### The limiter

- Detects true peaks (4x oversampled, 12-tap windowed sinc) so the ceiling holds after a DAC or an MP3/AAC decoder, not just on sample values.
- 5 ms lookahead; the lowest wanted gain over the span is smoothed with two moving averages whose total width equals the span, which is what guarantees the ceiling holds while gain reduction glides in instead of slamming.
- Release is program dependent: 40 ms after an isolated peak, up to 600 ms while the music is being limited continuously, so sustained limiting does not pump.
- One gain for all channels (no stereo image shift). Below the ceiling the signal passes through untouched (gain exactly 1.0).
- A small safety margin (0.05 dB) sits under the requested ceiling.

### Format

The output has the **same sample format as the input**: 16-bit in, 16-bit out (TPDF dither of one step); 24-bit in, 24-bit out. Nothing is padded to a bigger format. Samples the chain did not change come back bit-for-bit (no dither is added to an untouched signal).

### Hearing it in the editor

The editor has a **Loudness** tab of its own (next to Format & quality and Name, folder & tags). With a mode chosen and "Hear this when playing in the editor" ticked, the preview plays through the same gain curve and limiter as the export, so seeking to any spot (say 2:12:32) and listening tells you what the export will sound like. Untick it to compare with the original; the export keeps the setting either way.

How it works: each recording is measured once, in full (about 17 s for 3 hours, with a progress bar), and its profile is cached in memory and on disk (under 1 MB per 3 hours). The profile of the edit list is then stitched from the cached ones and the gain curve planned in about 10 ms, so trims, splits and re-orders are heard processed straight away. Playback runs the gain curve and limiter on the decoded audio; a seek starts the limiter a few milliseconds early so it has settled and every frame still lands where it is on the timeline.

The preview is within about half a LU of the export: the export makes one more check pass (the rehearsal) that the preview skips. Playback does not include fades, as before.

### Settings (Export window and Settings > Default export)

| Choice | Meaning |
|---|---|
| Leave as recorded | Off. Exact audio. |
| Match a loudness | Target LUFS (presets −14 / −16 / −11), "Evening out" none / gentle / medium / strong, peak limit in dBTP. |
| Raise to a peak limit | One steady gain so the highest true peak just meets the limit. Nothing else changes. |

| Evening out | strength | max turn-up / turn-down | slow build flattened |
|---|---|---|---|
| none | 0 | 0 | 0 |
| gentle | 0.4 | 3 dB | 0 |
| medium | 0.6 | 5 dB | 0.2 |
| strong | 0.85 | 8 dB | 0.5 |

For MP3 and AAC use a limit of −1 dBTP or lower: lossy encoding creates new overshoots after the limiter. For AIFF −0.1 is fine.

## Measured

`Serato Recording 1.aif` (3 h, 44.1 kHz, 16-bit; −19.6 LUFS, 4.6 LU range, peaks to −1.8 dBTP), target −14 LUFS, ceiling −1 dBTP, medium:

| | integrated | range | true peak |
|---|---|---|---|
| before | −19.6 LUFS | 4.6 LU | −1.81 dBTP |
| after | −14.1 LUFS | 4.0 LU | −1.05 dBTP |

Static gain +5.6 dB; the leveller moved it between +3.4 and +9.0 dB. The limiter averaged 0.01 dB, reached 4.4 dB at most, and was working 1.4 % of the time. The loud stretches at 2:12:32 and 2:14:01 came down from +5.4 / +4.0 LU above their surroundings to +3.4 / +2.8; breakdowns kept their depth (−13.8 → −13.6 LU).

Cost: measuring 17 s (about 650x realtime), each rehearsal about 17 s, the real pass about 39 s including the write; **15.6 MB peak memory**, independent of length. Everything is streaming, so time grows linearly with the total length of the combined set; a longer set made of several recordings simply reads each one the same number of times.

## Trying it on Serato Recording 1

Times are `h:mm:ss`. Found with `cargo run --release -p dynamics --example analyze -- "Serato Recording 1.aif"`.

| Listen at | What it is | What a good result sounds like |
|---|---|---|
| **2:09:00 – 2:15:00** | The most interesting six minutes: a loud stretch (2:09:31, +4.2 LU), a deep breakdown (2:11:47, −12 LU), a quieter passage (2:11:45 – 2:12:23, −4 LU), then the loudest stretch of the set (2:12:32 – 2:13:02, +5.4 LU) and a bass-heavy one (2:14:01, 81 % bass) | Loud stretches a couple of dB closer to the rest; the breakdown still clearly a breakdown; the quiet passage lifted; no pumping |
| 0:04:05 – 0:04:13 | Deep breakdown (−13.8 LU) | Must stay quiet |
| 0:05:37 – 0:05:53 | Loud, bass-heavy (+2.7 LU, 61 % bass) | Turned down slightly |
| 1:01:00 – 1:01:20 | Highest peak of the set (−1.8 dBFS), the limiter's hardest work | Clean, no crackle or dullness on the transients |
| 1:22:53 – 1:23:15 | Loud stretch (+2.8 LU) | Turned down slightly |
| 2:29:09 – 2:29:42 | Quiet stretch (−3.9 LU) | Lifted |
| 2:31:30 – 2:31:54 | Loud stretch (+3.9 LU) | Turned down |

Nothing in this recording reaches full scale, so there is no existing clipping to worry about. To hear it without the app:

```
cargo run --release -p dynamics --example maximize -- "Serato Recording 1.aif" out.aif --lufs -14 --ceiling -1 --level medium
```

and compare `out.aif` with the original at the times above (level-match by ear or by loudness before judging; the output is about 5.6 dB louder).

## The committed test set

`tests/fixtures/beeps-set.aif` (4 MB, 46 s, 22.05 kHz stereo 16-bit) is a made-up set of kicks, bass notes, beeps and hats, so there is nothing to license. `dynamics::synth` renders it deterministically, and a test fails if the committed file drifts from the generator (regenerate with `cargo run --release -p dynamics --example make_fixture -- --write`). It is built to exercise every behaviour:

| Time | What | Checked by |
|---|---|---|
| 0 – 8 s | Track A alone (reference level) | |
| 8 – 16 s | Beatmix: A fades out as B fades in, kicks aligned, basslines adding (+4.8 LU) | beatmix pulled toward the rest |
| 16 – 24 s | Track B alone, 5 dB low | quiet track lifted |
| 24 – 30 s | Breakdown: pads and hats only | stays a breakdown |
| 30 – 38 s | Drop: hotter, with sharp clicks (high peak-to-loudness) | ceiling holds, limiter works |
| 38 – 40 s | Silence | stays exactly silent (no dither on zeros) |
| 40 – 46 s | Track A again | |

It opens in the app like any recording, so the whole feature can be tried by ear in seconds: copy it into a folder the app lists.

## Not done yet

- **Bass control.** The profile already records the energy below 150 Hz. The idea is to turn down only the *excess* bass when two basslines add in a beatmix (a slow dynamic low shelf), instead of lowering the whole mix. On this recording most of the energy is bass, so the leveller already acts mostly on it; whether bass-only control is audibly better is for listening to decide. Escalation path: a 2-band linear-phase split.
- **Loudness lane on the timeline** showing short-term loudness and the planned gain before exporting.
- **Speed.** Measuring could be cached per source file, and the passes could use several threads. Not needed yet: a 3 h set takes about 1.5 minutes end to end.
