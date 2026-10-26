# Recording Fixer for Serato DJ

A small, free, open-source desktop utility (Windows and macOS) for fixing broken recordings made with Serato DJ and combining those recordings. Licensed under the [GNU GPL v3 or later](LICENSE). It is not affiliated with or endorsed by Serato; "Serato" and "Serato DJ" are their trademarks, used here only to say what the tool works with.

Found a problem? Please [open an issue](https://github.com/djryanj/recording-fixer/issues). The app shows its version in the bottom bar and under Settings > About.

- **Finds** your recordings in Serato's `Recording` and `Recording temp` folders (or any folder you add).
- **Repairs** `.aif` files Serato never closed properly (the ones ffmpeg and most players can't read).
- **Plays** recordings and your edits, to the output you choose, including **ASIO** drivers and any pair of output channels.
- **Edits visually**: a waveform timeline where you trim by dragging clip edges, split at the playhead, remove sections, and reorder clips. Non-destructive: nothing is written until you export.
- **Combines** several recordings into one, or splits one into several files.
- **Exports** to AIFF, MP3 or AAC (.m4a), with choice of bitrate mode and quality, **fade in / fade out**, ID3/MP4 tags, and filename templates.

Early version. VST3 support and mixed-sample-rate joins are not built yet; see [docs/FEATURES.md](docs/FEATURES.md).

## What goes wrong with Serato recordings

Serato writes an AIFF header with zeroed placeholders (file size, number of sample frames, audio size) and fills them in when you stop the recording. If Serato exits first, the audio is complete but those three numbers stay `0`, so players see an empty file. The repair patches those fields; it doesn't re-encode anything, so it is fast and lossless. See [docs/ANALYSIS.md](docs/ANALYSIS.md) for the byte-level details.

## Using it

1. Start the app. It lists recordings from the default Serato folders (`<Music>/_Serato_/Recording` and `Recording temp`). Use **Add folder…** for others.
2. Files that need fixing show **Needs repair**. A file that Serato may still be writing shows **In use** and is never touched.
3. Select files, then:
   - **Repair…** saves a repaired copy next to the original (default), or fixes in place (the original header is saved as `<name>.aif.hdrbak`).
   - **Export…** (or **Combine / Export…** with several files) opens the editor. Broken files can be exported directly without repairing them first.
4. In the editor:
   - The **timeline** shows every clip's waveform end to end, ordered by recording time. Click it to move the **playhead** (and the audio jumps there); drag a clip's edges to trim; drag a clip sideways to reorder; Ctrl+wheel zooms and the wheel scrolls.
   - **Space** plays and pauses what you are hearing, including your edits as you make them. The transport shows the position, level meters, volume and the current output.
   - **Split at playhead** cuts a clip in two (then **Remove** the part you don't want, which is how you cut out a section). **Trim start/end here** drops everything before/after the playhead. Keys: `S` split, `[` `]` trim, arrows move the playhead (Shift ±10 s, Alt ±0.1 s), `Delete` removes.
   - Choose **Combine into one file**, or **One file per clip** (handy for splitting a long set into tracks).
   - **Format & quality**: AIFF (lossless); MP3 as CBR (96-320 kbps), VBR (V0-V9) or ABR, with ID3v2.3/2.4 and optional ID3v1; AAC as CBR or VBR with a choice of encoder.
   - **Fades**: a fade in over the start and a fade out over the end of each exported file, in seconds, for every format. The timeline shows the fade as a dimmed ramp. (The fade is applied on export; playback preview does not include it yet.)
   - **Name, folder & tags**: a filename template such as `{date} {time} - {name}`, the output folder, and tags (title, artist, album, album artist, genre, date, track, comment). Tag fields accept the same placeholders: `{name}` `{date}` `{time}` `{clock}` `{datetime}` `{year}` `{n}` `{n2}` `{count}` `{ext}` `{quality}`. Date and time come from the recording's own timestamp. Your choices are remembered.
5. MP3/AAC need ffmpeg. The app finds it next to the program, on your `PATH`, or via **Locate ffmpeg…** (so you can point to your own build, for example one with extra codecs).

Limits: files being combined must share sample rate, bit depth and channel count (resampling is planned). An AIFF result can't exceed 4 GB; export to MP3/AAC for longer sets.

## Audio output and ASIO

Click **Output** in the transport to choose where audio plays. Devices are grouped by audio API:

- **ASIO**: low latency, and what DJ interfaces like the Pioneer DJM-S7 or DDJ-SX provide. Pick an **output channel pair** (for example 3-4 for a booth or headphone output) and an optional **buffer size**. Most ASIO drivers serve one program at a time, so close Serato (or choose another output) if the device will not open; the app shows the driver's own message.
- **WASAPI** (Windows) / **CoreAudio** (macOS): the shared system audio path.

The choice is remembered and reopened at start-up; if the device has gone, the app falls back to the system default. Files whose sample rate differs from the device's are resampled (FFT resampler) for playback. Exports are never resampled.

ASIO is a build option on Windows (`asio`, on by default): it needs **LLVM** (`winget install LLVM.LLVM`, and set `LIBCLANG_PATH` to its `bin` folder) because the build generates bindings to Steinberg's ASIO SDK, which cpal downloads. **Licensing:** that SDK is dual-licensed GPLv3 / proprietary, so an app built _with_ ASIO is subject to those terms if you distribute it. Build without it using `npm --prefix ui exec -- tauri build -- --no-default-features`; macOS builds always do.

## Apple's AAC encoder (optional, better quality than ffmpeg's built-in)

The app picks the best AAC encoder available, in this order: Apple (see below), `libfdk_aac`, ffmpeg's built-in `aac`, then Windows Media Foundation (`aac_mf`). The footer shows what was found, and the Format tab lets you choose any available encoder.

**macOS.** Nothing to download: Apple's encoder is part of the OS. Use an ffmpeg built with AudioToolbox (Homebrew's usually is). Check with:

```sh
ffmpeg -hide_banner -encoders | grep aac_at
```

If your ffmpeg lists `aac_at`, the app uses it automatically. If not, use **Locate ffmpeg…** to point at one that does.

**Windows.** Apple does not offer a standalone download of its encoder, and ffmpeg cannot use it on Windows. It lives in `CoreAudioToolbox.dll`, part of _Apple Application Support_, which Apple installs together with **iTunes**. That is also how OBS Studio gets its "CoreAudio AAC" encoder. To have it available:

1. Install iTunes for Windows from Apple or the Microsoft Store.
2. The app looks for the 64-bit `CoreAudioToolbox.dll` in `C:\Program Files\Common Files\Apple\Apple Application Support\` and `C:\Program Files\iTunes\`. When it loads and can create an encoder, the footer reads **"Apple AAC (CoreAudioToolbox)"** and **Apple AAC (CoreAudio)** becomes the default AAC encoder.

**How it works:** the app loads the DLL at runtime (as OBS does) and drives Apple's `AudioConverter` API directly: constant bitrate (up to 320 kbps) or Apple's true VBR in five quality levels. The AAC packets are wrapped in ADTS and ffmpeg muxes them into an `.m4a` without re-encoding, so tags work as usual. It encodes at about 100x realtime. Apple's encoder prepends 2112 samples (about 48 ms) of silence ("priming") that players are meant to skip. The app reads that value from the encoder (as OBS does) and has ffmpeg record it as an edit list, so players that honour it start exactly on time; a test checks that a click 0.5 s into the source decodes at 0.5 s to within 2 ms.

We never bundle or download Apple's DLL; don't fetch it from "DLL download" sites. If the DLL is missing or won't load, the app falls back to the next-best ffmpeg encoder. On Windows with 32-bit-only Apple software the 64-bit DLL won't be present and Apple AAC is unavailable.

You can also get better-than-default AAC by supplying an ffmpeg build that includes `libfdk_aac` (non-free, so it can't be bundled with this app) via **Locate ffmpeg…**.

## Developer setup

You need the following on your machine. Docker is not required: develop natively, because this is a desktop GUI app.

### Windows 11

Run in PowerShell, then **close and reopen the terminal** so `PATH` updates.

```powershell
# C++ build tools (needed by Rust's MSVC toolchain; several GB, may prompt for admin)
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup -e          # Rust (stable, MSVC)
winget install --id OpenJS.NodeJS.LTS -e        # Node.js 22+ (24 LTS recommended)
winget install --id Gyan.FFmpeg -e              # ffmpeg + ffprobe, for export and tests
```

WebView2 is preinstalled on Windows 11. On Windows 10, install the Evergreen WebView2 runtime.

For **ASIO** output (on by default on Windows) you also need LLVM, because the build generates bindings to Steinberg's ASIO SDK (downloaded automatically):

```powershell
winget install --id LLVM.LLVM -e
setx LIBCLANG_PATH "C:\Program Files\LLVM\bin"    # then reopen the terminal
```

Skip it with `--no-default-features` on the cargo commands (see the licensing note under _Audio output and ASIO_).

### macOS

```sh
xcode-select --install
# Homebrew: https://brew.sh
brew install rustup node ffmpeg
rustup default stable
```

### Verify

```sh
rustc --version && cargo --version && node --version && npm --version
ffmpeg -version && ffprobe -version
npm --prefix ui install && npm --prefix ui exec -- tauri --version   # the Tauri CLI comes with the UI dependencies
```

If `cargo` can't find `link.exe` on Windows, the Build Tools C++ workload didn't install; rerun the first command.

### Install `just` (task runner)

Day-to-day commands go through [`just`](https://github.com/casey/just), a small cross-platform task runner (Windows has no `make`). It is a **developer prerequisite**: install it once, then run `just` in the repo root to list the recipes.

```powershell
winget install --id Casey.Just -e     # Windows (reopen the terminal afterwards)
```

```sh
brew install just                     # macOS
```

| Recipe              | Does                                                                                       |
| ------------------- | ------------------------------------------------------------------------------------------ |
| `just setup`        | install UI dependencies (once)                                                             |
| `just dev`          | run the app with hot reload                                                                |
| `just check`        | `cargo fmt --check`, clippy `-D warnings`, svelte-check; must be clean                     |
| `just fmt`          | apply formatting                                                                           |
| `just test`         | Rust tests, then UI tests                                                                  |
| `just test-samples` | also the tests that need the real `.aif` samples in the repo root                          |
| `just build-debug`  | quick debug exe, no installer                                                              |
| `just e2e`          | build the debug exe, then run the end-to-end suite (Windows; one-time setup below)         |
| `just e2e-diagnose` | start the debug exe through msedgedriver with a verbose log (for session failures)         |
| `just release`      | optimised standalone app for this machine, no installer (`target/release/recording-fixer`) |
| `just build`        | installers                                                                                 |
| `just ci`           | `check`, `test` and `e2e`: what CI runs                                                    |

The recipes set `LIBCLANG_PATH` for ASIO on Windows and leave ASIO out on macOS, so you don't have to. They are thin wrappers; the raw commands are listed below if you prefer them.

### Run, test, build

```sh
npm --prefix ui install              # once; also installs the Tauri CLI (a UI dev dependency)
npm --prefix ui exec -- tauri dev    # run the app with hot reload
cargo test --workspace               # Rust tests (ffmpeg tests skip if ffmpeg is missing)
npm --prefix ui test                 # UI logic and component tests (Vitest)
npm --prefix ui run check            # svelte-check / TypeScript
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
npm --prefix ui exec -- tauri build  # installers (NSIS/MSI on Windows, .dmg on macOS)
```

### Testing

There are four layers, from fast to realistic. All of them run in CI (`.github/workflows/ci.yml`).

| Layer                 | What it covers                                                                                                                                            | Run                       |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| Rust unit/integration | Header parsing and repair, edit lists, fades, export arguments and real ffmpeg/Apple encodes, peaks, the playback engine against a simulated audio device | `cargo test --workspace`  |
| UI logic              | Timeline model, filename templates, export options, formatting                                                                                            | `npm --prefix ui test`    |
| UI components         | The real Svelte screens (list, editor, transport, audio settings, timeline pointer interaction) with a scripted fake backend                              | `npm --prefix ui test`    |
| End to end            | The **real built app**: real window, Rust backend and files. Repairs a broken file, combines, fades, exports MP3 with tags, previews audio                | `npm --prefix ui run e2e` |

The end-to-end tests drive the app through Edge WebDriver (msedgedriver starts the WebView2 app itself), using synthetic recordings in a temp folder (via `RECORDING_FIXER_FOLDERS`) and a throwaway WebView2 profile, so your Serato library is never touched. One-time setup on Windows:

```powershell
# msedgedriver must match the installed WebView2 runtime version:
$v = (Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}').pv
New-Item -ItemType Directory -Force ui/e2e/.tools | Out-Null
Invoke-WebRequest "https://msedgedriver.microsoft.com/$v/edgedriver_win64.zip" -OutFile ui/e2e/.tools/edgedriver.zip
Expand-Archive ui/e2e/.tools/edgedriver.zip ui/e2e/.tools/edgedriver -Force
npm --prefix ui exec -- tauri build --debug --no-bundle    # the app under test; rebuild after changing code
npm --prefix ui run e2e
```

Set `EDGEDRIVER` or `APP_EXE` to use other locations. The macOS equivalent (WKWebView has no WebDriver) is not set up; the component tests cover the UI there.

### Real sample files

The tests that use real recordings are marked `#[ignore]` because the files are large and private. Put `Serato Recording 1.aif`, `2.aif` and `5.aif` in the repo root (they're gitignored) and run:

```sh
cargo test --workspace -- --ignored
```

`tests/fixtures/serato-broken-head.aif` is a 1 MB slice of a real broken recording and is committed, so the main repair path is always tested.

### Project layout

```
crates/aiff-core/   parse, diagnose and repair AIFF headers; in-use detection; ID3 trailer
crates/edit-core/   non-destructive edit list, concatenation, fades, AIFF writer (streams, bounded memory)
crates/export/      ffmpeg discovery, encoder detection, command building, PCM piping, tags; Apple AAC (CoreAudioToolbox) on Windows
crates/library/     Serato folder discovery and the file list model
crates/peaks/       streaming waveform peaks (multi-resolution) with an on-disk cache
crates/playback/    audio engine: decoder thread, lock-free ring buffer, resampling, device backends (WASAPI, ASIO, CoreAudio)
src-tauri/          thin Tauri command layer
ui/                 Svelte + TypeScript front end (ui/src/lib: components and logic, ui/e2e: end-to-end tests)
docs/               ANALYSIS (design), FEATURES (scope), PERF (measured baselines)
```

See [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. [CLAUDE.md](CLAUDE.md) has the engineering rules (TDD for the core, streaming I/O and memory budgets, never modify user files by default).

## Loudness (optional)

When exporting you can choose **Match a loudness** (for example −14 LUFS for streaming) or **Raise to a peak limit**. It brings the whole set to the target, turns down stretches that are much louder than their surroundings (two tracks beatmixed) and turns up a track that was left too quiet, keeps breakdowns quiet, and uses a true-peak limiter so nothing goes over the limit. The output keeps the recording's own bit depth. It is off unless you choose it, because it changes the audio. Details, measurements and suggested listening spots are in [docs/MAXIMIZER.md](docs/MAXIMIZER.md).

## Privacy and logs

The app collects no data, ever; see [PRIVACY.md](PRIVACY.md). Its only network use is an optional once-a-day check of GitHub's public release list for a newer version (switch it off in Settings). When something goes wrong, **Settings > Troubleshooting** shows the local log file or copies it, so you can attach it to a bug report; it is never sent anywhere.

## License

Free software, **GPL-3.0-or-later**: see [LICENSE](LICENSE). You may use, study, change and share it; anything you distribute based on it must stay under the same license with its source available. It is not for sale and never will be.

Third-party pieces and how they fit (details in [docs/LICENSING.md](docs/LICENSING.md)):

- **ASIO** support uses Steinberg's ASIO SDK, which is available under the GPLv3: that is why this project is GPL. _ASIO is a registered trademark of Steinberg Media Technologies GmbH._
- **Fonts** (Barlow, Barlow Condensed, JetBrains Mono) are bundled under the SIL Open Font License 1.1.
- **ffmpeg** is a separate program the app runs, not part of it. Whoever packages a build with ffmpeg inside must follow ffmpeg's license (use an LGPL build, include its license text and a link to its source).
- **Apple's AAC encoder** is only ever loaded from the user's own iTunes install; this project never ships or downloads it. Never bundle `libfdk_aac` either.
- Rust crates and npm packages are MIT, Apache-2.0, ISC, BSD, Zlib, Unicode, or MPL-2.0 (five small crates), all compatible with the GPL.
