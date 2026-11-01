# Analysis: toolchain, environment, approach

## 1. What's actually wrong with the sample file

`Serato Recording 5.aif` (220,484,662 bytes), header decoded:

| Field | Value | Should be |
|---|---|---|
| `FORM` chunk size (offset 4) | `0` | file size − 8 |
| `COMM` format | 2 ch, 16-bit, 48 000 Hz (big-endian PCM) | fine |
| `COMM.numSampleFrames` (offset 22) | `0` | (file size − 54) / 4 = 55,121,152 frames (~19m 5s) |
| `SSND` chunk size (offset 42) | `0` | data bytes + 8 |
| SSND data offset / blockSize | 0 / 0 | fine; audio starts at byte 54 |

Serato writes a header with placeholder zeros and back-patches the sizes on a clean stop. If the app exits first, the audio is complete but the sizes are never written. ffmpeg sees a zero-length file.

**The fix is a 3-field patch** (plus trimming a partial trailing frame, if any). It needs no re-encoding and no ffmpeg, runs in milliseconds, and can be done in place or as a copy. The heavy lifting is the UI, not the repair.

Caveats to verify with more samples (we have only one):
- Whether 3-hour auto-split chunks are closed properly. Treat as unknown. The detector should compare header sizes against file length for *every* file instead of assuming.
- Whether other bit depths (24-bit) or sample rates appear. The fixer must read COMM, not hard-code 16/48k.
- A file Serato is still writing looks "broken" too. Detect an open handle or a recent mtime and refuse to touch it.
- Samples look L==R duplicated in this file (mono source in a stereo container). Irrelevant to the fix, but useful as a test of "don't assume".

## 2. Requirements that drive the stack

1. Windows + macOS, ideally one codebase.
2. File list first; waveform editor second.
3. Waveform view over files of ~1–3 GB: needs a precomputed min/max peak cache, not decoding on draw.
4. ffmpeg only for mp3/aac export (and resampling edge cases). AIFF repair and same-format concatenation are pure byte work and should be done in-process.

5. **Speed and memory are paramount.** Inputs reach several GB. This favours a native Rust core (no GC, memory-mapped and streamed I/O, predictable footprint) with a thin web UI that only ever receives pixel-sized binary peak buffers. It also argues against Electron's heavier baseline. See the budgets in CLAUDE.md.

## 3. Stack options

| Option | List/UI | Waveform | Byte-level file work | Verdict |
|---|---|---|---|---|
| **Tauri 2 + Rust core + Svelte/TS UI** | Excellent (web UI) | Canvas, fine | Excellent (Rust, mmap, safe) | **Recommended** |
| Electron + Node | Excellent | Fine | OK | Heavier (~150 MB), no advantage over Tauri |
| JUCE (C++) | Adequate, dated look | Good, built in | OK, more footguns | A C++ audio toolkit; no advantage for a list-first utility |
| Python + Qt | Good | OK | OK | Packaging on mac/win is the pain |
| .NET MAUI / Avalonia | Good | OK | Good | No advantage |

**Recommendation: Tauri 2 (Rust backend) + Svelte + TypeScript.**
- The repair, concat and peak-cache code is where correctness matters. Rust suits it, with `cargo test`, no GC pauses and cheap memory-mapped I/O.
- Tauri's installers are small, and it uses WebView2 (preinstalled on Windows 11) and WKWebView on macOS.
- A list-heavy UI is easiest on the web stack. The waveform view is a `<canvas>` fed pre-reduced peaks.
- ffmpeg is invoked as a subprocess (Tauri "sidecar", or found on PATH). We do **not** link it. This avoids LGPL/GPL redistribution questions: ship an LGPL build, or let the user point to their own.

## 4. Native Windows vs devcontainer

**Develop natively on Windows. Use a container only for the headless core, and CI for macOS.**

- This is a desktop GUI app. A Linux devcontainer cannot show the Tauri window without WSLg/X11 forwarding, and it cannot run WebView2, so you would not be testing the real thing. Audio-device and file-dialog behaviour also differs.
- It can't test macOS either. Only a macOS runner can. **GitHub Actions matrix (windows-latest, macos-latest)** is the right answer for cross-platform confidence.
- A devcontainer *is* useful for the pure-Rust core crate (`serato-core`: repair, concat, peaks, EDL) plus a pinned ffmpeg, as a reproducible test env. Optional, and low priority.
- Current machine state: **only Docker and git are installed.** Missing: Rust (rustup), MSVC Build Tools (C++ workload), Node 20+, ffmpeg. The installs are `winget install Rustlang.Rustup Microsoft.VisualStudio.2022.BuildTools OpenJS.NodeJS.LTS Gyan.FFmpeg`.
- The project is not a git repository yet. `git init` first.

## 5. Architecture

```
serato-fixer/
  crates/
    aiff-core/      pure Rust, no I/O policy: AIFF/AIFC header parse, diagnose, repair, PCM read
    edit-core/      EDL (edit decision list): clips, trims, gaps, crossfades; render plan
    peaks/          min/max pyramid builder + on-disk cache (.peaks next to app cache, keyed by path+mtime+size)
    export/         ffmpeg command builder + progress parsing; later: fx stage
  src-tauri/        thin command layer: scan, diagnose, repair, build_peaks, export; events for progress
  ui/               Svelte + TS: list view, waveform view, export dialog
  tests/fixtures/   generated synthetic files + a 1 MB head slice of the real broken file
```

Key design decisions:
- **Non-destructive by default.** Sources are never modified unless the user picks "repair in place" (and then a `.bak` is optional but defaulted on). A combine is an **EDL** saved as a small JSON project: ordered clips (file, in, out, gain, fade), nothing rendered until export.
- **Same-format concat is a byte copy** with a freshly written header. Mixed sample rates or bit depths go through ffmpeg (`-filter_complex concat` + `aresample`).
- **AIFF's 4 GB limit** (32-bit chunk sizes): a long combined output can exceed it. For `.aif` output beyond ~4 GB, warn and offer FLAC/WAV(RF64)/mp3/aac instead. Never silently write a truncated header.
- **Peak cache** at several zoom levels (e.g. 256, 1024, 4096 samples/bucket). The UI requests a range and a pixel width, and Rust returns the right level.
- **Default folder discovery**: Windows `%USERPROFILE%\Music\_Serato_\{Recording,Recording temp}`; macOS `~/Music/_Serato_/{Recording,Recording temp}` (same layout, believed but unverified on Mac). Serato also creates `_Serato_` at the root of external drives, so scan mounted volumes too. User-added folders are persisted.

## 6. Testing / TDD

TDD works well for the core and poorly for the pixels. Split accordingly:

- **Strict TDD (red/green) for crates:** header parse, diagnosis, repair, odd-length/truncated tails, 16/24-bit, mono/stereo, AIFC, EDL math, concat, peaks, ffmpeg argument builder. All are pure functions over bytes. Test fixtures are *generated* in code (synthetic sine files with a deliberately zeroed header), so there is no big binary in the repo.
- **Golden test on the real file:** a ~1 MB head slice of the real sample (the header bug is in the first 54 bytes) is committed as a fixture. The full 220 MB file stays out of git and is used by an opt-in `cargo test -- --ignored` test.
- **ffmpeg integration tests:** repaired output must satisfy `ffprobe` (duration within 1 frame of expected) and survive a decode. Gated on ffmpeg being present.
- **UI:** Vitest + Testing Library for list/selection/export-dialog logic; keep the waveform component thin and driven by the tested core. Add a couple of Tauri WebDriver smoke tests later; don't chase pixel TDD.

## 7. Risks / open questions

1. Only one sample file: gather 2–3 more (24-bit, an auto-split 3 h chunk, a Mac one).
2. Not-yet-written files while Serato is running (handled by a lock/mtime check).
3. Serato's recording folder semantics: does "Recording temp" hold in-progress or orphaned files? Confirm on a real session.
4. ffmpeg distribution: bundled LGPL build vs. user-supplied. Decide before the first release.
