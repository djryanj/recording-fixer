# Performance baselines

Speed and memory are first-class requirements (see CLAUDE.md). This file records measured numbers so regressions are visible. Treat a regression of more than ~10% as a bug.

Machine: Windows 11, NVMe SSD, release build, files warm in the OS cache unless noted. Re-measure on a cold cache and on an external/spinning drive before quoting these as real-world.

## Budgets

| Area | Budget | Status |
|---|---|---|
| Resident memory during join/repair/export of a multi-GB file | < 150 MB, independent of file size | Join: 5.5 MB (met) |
| Same-format combine | near disk throughput | 2,582 MB/s warm cache |
| Scan: header + tail only, never audio | first rows < 1 s for 100 files | not yet measured |
| Waveform overview of a 3 h file | seconds | 0.94 s to build, 9.2 MB (met) |

## Measurements

### Join two recordings (Serato Recording 1 + 2, 44.1 kHz/16-bit, 2.75 GB total)

```
cargo build --release -p edit-core --example join_bench
target\release\examples\join_bench.exe out.aif "Serato Recording 1.aif" "Serato Recording 2.aif"
```

- Time: 1.1 s (2,582 MB/s)
- Peak working set: 5.5 MB
- ffprobe duration of the output: 15600.111746 s, exactly (476,280,064 + 211,684,864) / 44,100

Peak memory is measured by launching the example and polling `PeakWorkingSet64` every 50 ms until it exits.

### Repair + export of the real broken file (Serato Recording 5, 220 MB, 19 min)

`cargo test -p export --test real_samples -- --ignored` (debug build, includes two ffmpeg encodes): about 23 s total. Dominated by the mp3 and aac encodes, not by our code. Repaired file duration matches the header-derived expectation to within 10 ms.

### Waveform peaks (Serato Recording 1, 3 h, 1.9 GB)

```
cargo build --release -p peaks --example peaks_bench
target\release\examples\peaks_bench.exe "Serato Recording 1.aif"
```

- Build: 0.94 s for 476,280,064 frames (one streaming pass), 5.0 MB of peaks across 6 levels
- Peak working set: 9.2 MB
- Query: 2000-column overview in 25 µs; a 10 s zoom (1500 columns) in 13 µs
- Recording 5 (19 min, 220 MB): 0.12 s, 5.5 MB

The UI requests only the visible range, sized by pixel width, as raw bytes over IPC. Warm cache; re-measure cold.

### Encoding the real broken file (Serato Recording 5, 19 min audio, 220 MB)

```
cargo build --release -p export --example export_bench
target\release\examples\export_bench.exe "Serato Recording 5.aif" out.m4a apple   (or mp3 / aac)
```

| Encoder | Time | Speed | Our process peak memory |
|---|---|---|---|
| MP3 320 kbps CBR (libmp3lame via ffmpeg) | 9.8 s | 117x realtime | 5.5 MB |
| AAC 256 kbps CBR (ffmpeg built-in) | 14.9 s | 77x | 5.5 MB |
| AAC 256 kbps CBR (Apple CoreAudio, direct) | 11.1 s | 104x | 15.3 MB |

ffmpeg's own memory is not included in these figures. Audio is streamed; nothing proportional to file size is buffered.

### Loudness maximizer (Serato Recording 1, 3 h, 44.1 kHz stereo 16-bit)

```
cargo run --release -p dynamics --example maximize -- "Serato Recording 1.aif" out.aif --lufs -14 --ceiling -1
```

| Step | Time | Speed |
|---|---|---|
| Measure (K-weighted loudness, true peak) | 17 s | about 650x realtime |
| Rehearse (whole chain, output discarded), once or twice | about 17 s each | |
| Process and write | 39 s | about 275x |

Measuring Serato Recording 1 (3 h) as the app does it (`cargo test -p dynamics --test real_samples -- --ignored --nocapture`): 12.4 s in a development build (874x realtime), first graph data after 3 ms. Without the `[profile.dev.package.*] opt-level = 3` overrides in the root Cargo.toml the same code ran at 37x realtime in development builds (about 5 minutes), so keep those overrides for the audio crates (`dynamics`, `peaks`, `playback`, `edit-core`, `aiff-core`, `rubato`).

Peak working set 15.6 MB, independent of length. Time grows linearly with the total length of the set. See [MAXIMIZER.md](MAXIMIZER.md).

## To do

- `criterion` benchmarks for diagnose, repair copy and peak build.
- Automated peak-RSS regression test in CI.
- Scan benchmark over 100+ files, including on an external drive.
