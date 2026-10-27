# Recording Fixer for Serato DJ

Desktop utility (Windows + macOS) that finds Serato DJ recordings (`.aif`), repairs ones with broken headers, combines several into one via a non-destructive edit list, and exports to mp3/aac through ffmpeg.

Read first: [docs/ANALYSIS.md](docs/ANALYSIS.md) (decisions and rationale), [docs/FEATURES.md](docs/FEATURES.md) (scope and priorities).

## The core bug we fix

Serato writes AIFF headers with zeroed placeholders and back-patches them on a clean stop. If it exits uncleanly, the audio is intact but three fields stay 0:
- `FORM` chunk size at byte 4 (should be file size − 8)
- `COMM.numSampleFrames` at byte 22 (should be audio bytes / (channels × bytes per sample))
- `SSND` chunk size at byte 42 (should be audio bytes + 8)

Audio starts after the `SSND` header, at byte 54 when offset and blockSize are 0. All multi-byte values are **big-endian**; sample rate in COMM is an 80-bit extended float. Always parse COMM for the real format. Never hard-code 16-bit/48 kHz.

### What a properly closed file looks like (verified on samples 1 and 2)

- Header sizes are consistent: `SSND size = frames × blockAlign + 8`. `FORM size` is either `file size − 8` (sample 1) or stops at the end of the audio and **excludes the ID3 trailer** (sample 2: Serato appends the tag after finalising FORM). Both are healthy; ffmpeg-style readers ignore trailing data. The diagnosis only flags FORM when it is short of the audio or past EOF.
- After the audio there is a trailing **`ID3 ` chunk** (4,675 bytes in both samples) holding an ID3v2.3 tag: `TIT2` "Serato Recording", `TXXX SERATO_PLAYCOUNT`, `RVAD`, and a `COMM` comment like `Recorded 2025-10-04 10:…`. The tag is written only on a clean close, so **broken files have no trailer**, and the audio runs to EOF.
- Parsers must walk chunks and bound audio by `SSND`/frame count, never "to EOF" for a healthy file. The repair must not treat the ID3 chunk as audio. Read the `Recorded …` comment for session time and ordering hints.
- Sample rates vary between recordings: sample 5 is 48 kHz, samples 1 and 2 are 44.1 kHz. Combining files with different rates is a real case, not an edge case.
- Sample 1 is exactly 3:00:00 (476,280,064 frames at 44.1 kHz), ~1.9 GB, and was closed properly, so the 3-hour split writes a valid header. Still check every file.
- `Recording temp` holds in-progress and orphaned recordings. A file there is "in use" until proven otherwise; treat it as orphaned only if nothing holds it open and it is not recent.

## Export: AAC encoder policy

- Offer an **LGPL ffmpeg build** by default (bundled). Allow a **user-supplied ffmpeg** (e.g. one built with non-free codecs such as libfdk_aac).
- Detect available AAC encoders at startup via `ffmpeg -hide_banner -encoders` and pick the best one: Apple AudioToolbox (`aac_at`) > `libfdk_aac` > native `aac` > `aac_mf`. Show which is in use. Let the user override. (Implemented in `crates/export`.)
- **Apple AAC** (the approach and its sources are explained in comments in `crates/export/src/apple_aac.rs`):
  - macOS: AudioToolbox is part of the OS. Use ffmpeg `aac_at` if the build has it. `afconvert` is detected but not used. **Untested on a Mac.**
  - Windows: **implemented** in `crates/export/src/apple_aac.rs`. We load the 64-bit `CoreAudioToolbox.dll` at runtime (`libloading`, `LOAD_WITH_ALTERED_SEARCH_PATH` so its sibling DLLs resolve; once per process, never unloaded) and drive `AudioConverterNew/SetProperty/FillComplexBuffer`. Output is raw AAC-LC packets, wrapped in ADTS (`adts.rs`) and piped to ffmpeg, which muxes to .m4a with `-c:a copy`. Never bundled or redistributed. Exposed as the pseudo-encoder `apple_coreaudio`, first in preference when `probe` succeeds.
  - CBR sets `'acbf'` mode 0 then `'brat'`; VBR sets mode 3 then `'vbrq'` (levels 1-5 map to 23/45/63/91/127). The DLL's FourCC property IDs and struct layouts were written from memory of Apple's headers and validated by tests (valid ADTS, bitrate in range, VBR size increasing with quality, ffmpeg decodes it), not against Apple's headers.
  - **Encoder delay is compensated.** The encoder *prepends* `leadingFrames` (2112) of silence; measured: a click 0.5 s in decoded 48.1 ms late. Padding more silence would double the error (a wrong idea that came up). The fix: we read `kAudioConverterPrimeInfo` and pass `-itsoffset -<priming/rate>` to the mux step, so ffmpeg writes an edit list. Regression test `apple_aac_audio_lands_where_it_was_recorded` decodes a click and requires < 2 ms error.
  - If nothing Apple is found, fall back to the best ffmpeg encoder.
- **Export options** live in `crates/export/src/options.rs`: MP3 CBR/VBR(V0-V9)/ABR, ID3v2.3/2.4, optional ID3v1; AAC CBR or VBR level 1-5, mapped per encoder in `ffmpeg.rs::aac_args` (aac_at `-q:a`, fdk `-vbr`, native `-q:a`; `aac_mf` has no VBR so it approximates with CBR). Tags are `-metadata`. **Filename and tag templates are expanded in the UI** (`ui/src/lib/naming.ts`); the backend only sanitises the final stem.

## Stack and layout

- Tauri 2 with a Cargo workspace: `crates/aiff-core` (parse/diagnose/repair, in-use check, ID3), `crates/edit-core` (edit list, concat, AIFF writer, `pump`), `crates/export` (ffmpeg discovery/args/run, encoder + Apple detection), `crates/dynamics` (loudness maximizer, see below), `crates/library` (folder discovery and the file list model), `crates/peaks` (streaming min/max pyramid + disk cache), and a thin `src-tauri/` command layer.
- UI: Svelte 5 + TypeScript in `ui/`. The waveform timeline (`Timeline.svelte`) is a canvas that fetches only the visible range from the backend (`get_peaks` returns raw bytes, never JSON). Pure UI logic lives in `ui/src/lib/*.ts` with Vitest tests: `timeline.ts` (trim/split/remove/reorder), `naming.ts`, `exportOptions.ts`, `format.ts`, `library.ts`.
- Waveform data: peaks store the most significant byte of each big-endian sample (that byte *is* the i8 value), min/max per 256-frame bucket, 4x coarser per level, cached in `<app cache>/peaks/` keyed by path+size+mtime, pruned to 300 MB. Peaks of a 3 h file are ~5 MB and build in about 1 s.
- ffmpeg/ffprobe via **subprocess only** (never linked). Located via user setting, bundled copy beside the exe, then PATH. PCM is piped to ffmpeg's stdin.
- Keep the export pipeline staged: `read → loudness → fades → encode`, each stage a `Read` wrapping the one before.
- `crates/playback`: decoder thread -> lock-free ring (`ringbuf`) -> device callback (cpal). The device stream stays open and "paused" means the callback outputs silence; seeks/reloads set a `hold` flag (callback mutes instantly) then bump a generation counter that the callback acknowledges after flushing, so stale audio is never heard. Position is `base + frames_played/rate`. The device layer is behind the `Backend` trait with a simulated device (`testing.rs`) so the whole engine is tested without hardware. WASAPI and ASIO4ALL were verified on this machine; FL Studio ASIO opened but did not drive callbacks and Voicemeeter refused (their own engines were not running). ASIO needs LLVM + the `asio` feature (default for `src-tauri`); `--no-default-features` omits it. The ASIO SDK is GPLv3/proprietary dual-licensed; a distributed build with ASIO is therefore subject to GPLv3 or Steinberg's terms (undecided, flagged to the user).
- Timeline gestures (`Timeline.svelte`): click or ruler click = play from there; drag across a waveform = rubber-band zoom (`zoomToRange`); drag a clip's 18 px **name bar** = reorder (shown by a grip); drag an edge = trim. A press that moves < 6 px is a click. Zoom goes down to single samples (40 px per frame at most). Below 32 frames per pixel the timeline fetches **exact** 16-bit min/max per frame (`get_samples` → `peaks::read_samples`, at most 262,144 frames per call; wider samples keep their top 16 bits) and draws them as columns, then held-sample bars, then a joined line, then dots; coarser views use the 8-bit peak pyramid. Because the real timeline is billions of pixels wide at that zoom, `viewX` (real px) is the source of truth for panning and the native scroller is capped at 8 M px and mapped onto it (`scrollScale`). Clicks, Ctrl+Left/Right and `#N` start/end entries all land on whole sample frames (`snapToFrame`); the ruler switches to `#<sample>` labels under 10 ms. Very short material still cannot zoom past its fitted view if it fits first; e2e tests that need headroom use one-minute clips.
- **Loudness maximizer** (`crates/dynamics`, design and measured numbers in [docs/MAXIMIZER.md](docs/MAXIMIZER.md)): BS.1770 loudness profile (100 ms steps) -> leveller gain curve with whole-set lookahead (preserves breakdowns and the slow energy arc) -> true-peak lookahead limiter -> dither back to the **input's own bit depth** (never inflate the format). Export chain is `read -> Maximizer -> FadeReader -> encode`; fades go after it so the leveller cannot lift a fade-out. `prepare()` measures then rehearses the chain (output discarded) to correct the gain onto the target; the real pass is a `Read` adapter. Off by default (it changes the audio); with a no-op setting the audio is bit-exact. Streaming, 16 MB peak memory on a 3 h file. Test set: `tests/fixtures/beeps-set.aif` is generated by `dynamics::synth` (a test fails if it drifts; regenerate with the `make_fixture` example). CLI tools: `--example analyze` (find interesting sections of a recording) and `--example maximize`. The editor preview plays through the same chain (`playback::Processing`: gain curve + limiter inside the decoder, restarted with pre-roll on every seek so frames stay exact); per-recording profiles are cached in memory and in `<cache>/loudness/*.lp`, and an edit list's profile is `Profile::stitch`ed from them (planning takes about 10 ms). The Loudness controls have their own editor tab (`LoudnessPanel.svelte`; also a section in Settings). The timeline has a **loudness lane** (short-term LUFS per recording, grey; after processing, amber; target dashed) shown while the Loudness tab is open or processing is on. Measurement is progressive: `maximizer::measure_with` publishes steps, `LoudnessStore.partial` serves them to `get_loudness`, `loudness-progress` events drive the dimmed frontier and the progress bar, and the editor plays the original until `ensure_loudness` finishes, then reloads processed. **Keep `[profile.dev.package.*] opt-level = 3` for the audio crates in the root Cargo.toml**: unoptimised, measuring a 3 h recording took 5 minutes in dev builds (12 s optimised); `dynamics/tests/speed.rs` guards it. Bass-only control is not built yet.
- Fades have their own editor tab (and a Settings section): `FadesPanel.svelte`.
- **Waveform builds are progressive**: `peaks::build_with` hands over finest-level buckets as each chunk is made; the backend keeps them in `PeaksStore.partial`, emits `peaks-progress`, and `get_peaks` answers from the partial data (never past what exists) while the build runs. The timeline draws the part that exists, dims the rest, marks the frontier and shows "building waveform N%" in the clip title, so a long file never looks frozen.
- **Logging** (`src-tauri/src/logging.rs`): own small `log` backend writing `recording-fixer.log` (+ `.1`, 1 MB each) in Tauri's `app_log_dir` (Windows: `%LOCALAPPDATA%com.ryanjjacobs.recording-fixerlogs`); also records panics with a backtrace. Commands log via `logged(...)`; messages carry file *names* only. The UI forwards uncaught errors through `log_ui` (`diagnostics.ts`). Settings > Troubleshooting shows the file or copies its tail. Level via `RECORDING_FIXER_LOG=debug`.
- **Update check** (`src-tauri/src/update.rs`, `ui/src/lib/updates.ts`): shells out to `curl` (no HTTP stack linked) for GitHub's public `releases/latest`, fixed User-Agent with no version/OS, `--disable` so no curlrc; refuses release links outside the repo; ignores drafts/pre-releases; 404 = no release yet. Runs at most daily at startup (switch in Settings, default on) and on "Check now"; shows a status-bar notice, never downloads. `RECORDING_FIXER_NO_UPDATE_CHECK` disables it (e2e sets it).
- **Privacy is a promise**: [PRIVACY.md](PRIVACY.md) says no data is collected, ever. No telemetry/analytics/crash upload; any new network access must be opt-out, identifier-free, and documented there in the same change.
- Top bar: a labelled "Show recordings from" row of real checkboxes (with plain hints and counts), and a "⚙ Settings" button (`SettingsDialog`: default export format/quality/fades, what ffmpeg is and where it was found, an informational best-first list of AAC encoders, and a link to the audio output window). The button shows a red LED and "needs ffmpeg" only when ffmpeg is missing. Settings and the export editor share one remembered store (`exportSettings.ts`, key `exportSettings.v1`); Settings writes only when something is changed. Keep wording plain: the user could not tell what bare `Recording` / `ffmpeg` controls were for.
- Fades live in `edit-core::fade` (quarter-sine, applied in-process to the PCM stream), so AIFF, ffmpeg and Apple output are identical. They apply to each exported file, not per clip. Preview playback does not include them yet.
- **Design system** (`ui/src/app.css`): DJ-booth instrument panel, deliberately not a generic dashboard. Warm graphite, one sodium-amber signal colour, hairline squared panels, condensed uppercase labels (Barlow Condensed), mono numerals (JetBrains Mono), LED status dots instead of pills, a large amber time readout; light theme is cream "studio paper" with burnt amber. Fonts are bundled via `@fontsource` (OFL). Theme is selectable (Auto/Dark/Light) and persisted. Keep new UI inside these tokens; do not reintroduce rounded cards, blue/teal accents or system-ui defaults.
- Gotchas learned:
  - The Tauri CLI runs `beforeDevCommand`/`beforeBuildCommand` from the directory that holds the nearest `package.json`; **a second `package.json` anywhere under the repo (e.g. `tests/e2e`) made it run the build in the wrong place**. The e2e suite therefore lives in `ui/e2e` and shares `ui/package.json`.
  - **Windows paths are case-insensitive**: `Timeline.test.ts` and `timeline.test.ts` are the same file (one overwrote the other once). Name component and logic tests differently (`TimelineView.test.ts` vs `timelineModel.test.ts`).
  - **Never set `resolve.conditions` outside test mode in `ui/vite.config.ts`**: it replaces Vite's defaults, the production bundle is built against Svelte's *server* entry, and the real app shows a blank window ("`mount(...)` is not available on the server") while every unit/component test still passes. Only the end-to-end test catches this.
  - **wry overrides `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`** by always setting its own WebView2 arguments, so on the CI runner msedgedriver's `--remote-debugging-port=0` was ignored (`DevToolsActivePort file doesn't exist`) while the same exe worked locally. The window is therefore built in code (`create_main_window`, `"create": false` in `tauri.conf.json`) and folds the variable into `additional_browser_args`. msedgedriver's `args` capability goes to the *app's* command line, not the browser. If e2e fails only on CI, run `just e2e-diagnose` / read the `e2e-logs` artifact.
  - **`Swatinem/rust-cache` does not save when the job fails**, and PR caches are only visible to that PR, so a job that keeps failing on main rebuilds from scratch on every run (the e2e job did: a 2-3 s restore is a miss). A flaky test on main therefore also costs every PR its cache. The same applies to `cargo install`ed tools, which is one reason CI installs none.
  - A function returned from a vitest `beforeEach` is a cleanup callback: `beforeEach(() => mock.mockReset())` calls the mock after each test. Use braces.
  - In Svelte grid layouts give children `min-width: 0` (and `minmax(0, 1fr)` tracks) or a wide canvas feeds its width back and overflows the window.
  - The timeline must be measured before it fits itself; fitting against the 800 px placeholder left the clips filling only part of the view.
  - Window screenshots from a non-interactive shell fail; check UI by serving `ui/dist` with a mocked `window.__TAURI_INTERNALS__` in headless Edge, or use the e2e driver's `takeScreenshot`.

## Commands

- The app version comes from `[workspace.package] version` in the root `Cargo.toml` (shown in the status bar and Settings > About via the app plugin; `tauri.conf.json` deliberately has no version). The GitHub URL lives in `ui/src/lib/about.ts` and the root `Cargo.toml`; the owner/repo in it was a guess, so correct it if the repository lives elsewhere.
- Releases: `just prepare-release X.Y.Z[-rc.N]` (branch + version bump via `scripts/release.mjs`, changelog closed only for final releases) then PR, merge, `just tag-release X.Y.Z`. The tag push starts `.github/workflows/release.yml` (build both OSes with the bundled ffmpeg, sign if configured, install the Windows installer and run the e2e suite on it, then draft the GitHub release); see docs/RELEASING.md. Each release also ships a `no-ffmpeg` variant of each installer (`src-tauri/tauri.release-lite.conf.json`, same code, no sidecar). Release builds use `src-tauri/tauri.release.conf.json` (ffmpeg sidecar + licence resources; `scripts/fetch-ffmpeg.mjs`, `scripts/build-ffmpeg-macos.sh`); dev/CI builds do not need the binary. User docs: docs/USER-GUIDE.md (linked from Settings > About): keep it in step with the UI.
- Shortcuts live in the `justfile` (`just check`, `just test`, `just e2e`, `just ci`, ...); `just` is a documented developer prerequisite (README "Install just"). Keep the README table and the justfile in step.
- `cargo test --workspace` runs the Rust tests (ffmpeg tests skip if ffmpeg is absent)
- `cargo test --workspace -- --ignored` runs tests needing the full sample files in the repo root
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` (both must be clean)
- `npm --prefix ui test` for UI tests (Vitest); `npm --prefix ui run check` for svelte-check / tsc
- `just dev` for the app; `just build` for installers; `just build-debug` for a quick exe. The Tauri CLI is the npm dev dependency `@tauri-apps/cli` (no `cargo install`); raw form `npm --prefix ui exec -- tauri ...`, which runs from the repo root (from `ui/` it cannot find `src-tauri`).
- On this machine `cargo`/`npm` are not on the PATH of fresh PowerShell tool shells; prepend `$env:USERPROFILE\.cargo\bin` and the Machine+User PATH.

## Testing

Four layers; a change is not done until the relevant ones pass. See README "Testing" for setup.
1. **Rust** (`cargo test --workspace`, clippy `-D warnings`, `cargo fmt --check`): strict TDD for the core crates; fixtures are generated in code; the playback engine runs against a simulated device and checks audio sample-for-sample (seeks, pauses, resampling, channel routing, device switching). Real-sample tests are `#[ignore]` (`-- --ignored`).
2. **UI logic** (Vitest, node): `timeline.ts`, `naming.ts`, `exportOptions.ts`, `format.ts`, `library.ts`, `theme.ts`, the audio store.
3. **UI components** (Vitest + jsdom + Testing Library, `// @vitest-environment jsdom`): every screen against `src/test/backend.ts`, a scriptable fake of all Tauri commands. The canvas gets a recording 2D context (`src/test/setup.ts`); `TimelineHarness.svelte` binds the zoom like the editor does.
4. **End to end** (`npm --prefix ui run e2e`): the real built app through msedgedriver directly (`ms:edgeOptions.binary`; no tauri-driver), synthetic recordings via `RECORDING_FIXER_FOLDERS`, throwaway `WEBVIEW2_USER_DATA_FOLDER`. After changing any code the exe must be rebuilt first (`just build-debug`) or the test runs the old app. It has caught a blank-window production regression that layers 1-3 could not.
When a test fails, ask whether the app or the test is wrong: both happened repeatedly (real bugs found this way: removing a clip threw and lost the selection; the timeline fitted to the wrong width; the resampler returned one frame too many from float rounding; a seek-then-play could blip old audio).

## Development environment

- Develop **natively on Windows**. Do not use a devcontainer for the app (GUI/WebView2 does not work in it). A devcontainer is acceptable only for the headless core crates.
- Requires Rust (rustup, MSVC target), VS 2022 Build Tools (C++ workload), Node 22+ (Vite 8 needs 20.19+ or 22.12+; CI uses 24), ffmpeg. See README.md for install commands.
- macOS is verified in CI (GitHub Actions matrix: windows-latest, macos-latest). Don't claim macOS behaviour works unless CI or a Mac confirms it.
- Shell on this machine is PowerShell/Git Bash. Use forward slashes in scripts that must run on both OSes.

## Performance and memory requirements (paramount)

Inputs are 0.2–3+ GB each, and users may have dozens. Speed and memory are first-class requirements, not polish. Every feature must meet them before it is considered done.

**Budgets** (targets; confirm and adjust once we have a benchmark baseline, and keep them in `docs/PERF.md`):
- Resident memory is **bounded and independent of file size**: aim for < 150 MB for the whole app while scanning, repairing, or exporting a 3 GB file. No code path may allocate proportional to file length.
- Folder scan with diagnosis of 100 files: list appears **progressively** in < 1 s for the first rows. Diagnosing a file reads only its header (first ~64 bytes) and the tail (to find the ID3 chunk); it must never read the audio.
- Repair: patch header fields in place or copy at disk speed (sequential, large buffers, e.g. 1–4 MB). The in-place patch of three fields is O(1) in file size.
- Combine/export of same-format PCM: **I/O-bound streaming copy**, at near disk throughput. No intermediate temp files unless unavoidable.
- Waveform: first usable overview of a 3 h file in a few seconds, then refine in the background. Zoom and scroll stay at 60 fps by reading only precomputed peak levels for the visible range.
- UI thread is never blocked. All file work runs off the main thread with cancellation and progress.

**Rules**
- Stream with fixed-size buffers or memory-map read-only. Never `read_to_end` or collect an audio file into a `Vec`. Peaks are computed in one streaming pass.
- Peak cache: multi-resolution min/max on disk, keyed by path + size + mtime; the UI fetches only the range and zoom level it is drawing. Cache is size-capped with eviction.
- Never send audio or large arrays over the Tauri IPC as JSON. Send reduced peak buffers as binary (typed arrays), and keep payloads sized by pixel width, not sample count.
- Do work in parallel where it helps (scan/diagnose, peak builds across files with a bounded thread pool). Keep concurrency limited so disk thrash and memory stay in budget, especially on spinning or external drives.
- Prefer ffmpeg only when needed; pipe PCM to its stdin instead of staging a full temp copy. Avoid decode→re-encode of PCM for repair and same-format joins.
- Handle backpressure: bounded channels between read, process, and encode stages.
- No per-sample allocation or dynamic dispatch in hot loops; process in blocks. Use `u64` offsets (files exceed 4 GB-adjacent sizes), and checked arithmetic on sizes.

**Verification**
- Benchmarks (`criterion`) for header diagnose, repair copy, concat, and peak build, using generated multi-GB sparse or synthetic files, and the real samples when present (opt-in).
- A memory test: run repair/combine/peaks on a large generated file and assert peak RSS stays under budget (measure in a child process; fail CI on regression).
- Record baseline numbers in `docs/PERF.md` and treat regressions beyond ~10% as bugs.
- Profile before optimising, but design for streaming from the start. Retrofitting is the expensive path.

## Packaging and distribution

- Goal: one download per platform. Tauri bundles: Windows NSIS `.exe` (and optionally `.msi`), macOS universal `.dmg`. Built by CI on `windows-latest` and `macos-latest`.
- ffmpeg/ffprobe (LGPL build) ship as Tauri **sidecars** (`bundle.externalBin`). Sidecar files must be named with the target triple, e.g. `binaries/ffmpeg-x86_64-pc-windows-msvc.exe`, `binaries/ffmpeg-aarch64-apple-darwin`. The user-supplied ffmpeg setting always overrides the bundled one.
- Not bundled: Apple's AAC encoder (user installs it). ffmpeg IS bundled in release builds (LGPL, see docs/THIRD-PARTY.md); ASIO is included by default.
- Installer must bootstrap WebView2 on Windows 10 (`webviewInstallMode`).
- Signing is a later decision. Unsigned builds trigger SmartScreen (Windows) and Gatekeeper (macOS, needs right-click → Open). Proper release needs a Windows code-signing cert / Azure Trusted Signing and an Apple Developer account for notarisation.
- Release checklist: include the LGPL license text and a link to the exact source/build of the bundled ffmpeg; list third-party licenses; verify the app launches on a clean machine with no dev tools.

## Working agreements

- **TDD for the core crates.** Write a failing test first, then the code. Fixtures are generated in code (synthetic sine files with zeroed headers). The 220 MB real sample `Serato Recording 5.aif` stays out of git (put it in `.gitignore`). A ~1 MB head slice is the committed golden fixture. UI is tested at the logic level; do not chase pixel TDD.
- **Never modify user recordings by default.** Repair writes a new file. In-place repair requires an explicit choice and creates a `.bak` unless turned off. Combine/cut is an edit list that is only rendered at export.
- **Never touch a file Serato is still writing** (open handle or very recent mtime). Treat this as a hard rule, with tests.
- **No silent truncation.** If AIFF output would exceed the 4 GB chunk limit, fail loudly or offer another format. Never write a wrapped size.
- Core crates do pure byte/data work. No Tauri imports, no global state, so they stay testable and reusable.
- Large files: see "Performance and memory requirements" above. Stream or memory-map; never read a multi-GB file into memory.
- Paths: use `PathBuf`/`Path`, never string concatenation. Handle spaces and non-ASCII (`Serato Recording 5.aif`, `_Serato_`).
- Default folders: `~/Music/_Serato_/Recording` and `~/Music/_Serato_/Recording temp`; also `_Serato_` at the root of external drives. The macOS path is the same layout but unverified.
- Keep comments sparse and explain *why*, not what. Match surrounding style. Run `cargo fmt`/`clippy` before finishing.
- The UI is list-first. The waveform/combine view is secondary but reachable in one click from the list.

## Open questions (do not assume)

- (Resolved) Licence: GPL-3.0-or-later, free and never sold; see `docs/LICENSING.md`. Do not add dependencies that are not GPLv3-compatible. Keep the Steinberg (ASIO) and Serato non-affiliation notices in the README and in Settings > About; nowhere else in the UI.
- Real ASIO hardware (DJM-S7, DDJ-SX) could not be tested: those drivers did not enumerate in this remote session. ASIO4ALL worked. Output-pair routing to channels 3-4+ is covered only by the simulated device.
- Playback does not include the fades. Peak/latency of the audio path (device latency is not added to the reported position) is not measured.
- The end-to-end suite runs on Windows only (locally and in CI, where it is part of the required "CI result"); macOS (WKWebView has no WebDriver) relies on the component tests.
- The timeline's pointer handling is tested with synthetic events in jsdom and (indirectly) the e2e suite; real mouse dragging has not been exercised by a person.
- Bit depth: the user says Serato does not produce 24-bit, but Serato's own support article says 24-bit is the default and splits at 1 h 50 min (16-bit: 3 h). All samples so far are 16-bit. The code reads depth from COMM and is tested with 24-bit synthetic files, so either way works. No real 24-bit sample is required.
- Still needed: a sample from macOS. Also, is a 3 h chunk's *successor* file tagged differently (ID3 comment) so we can auto-link splits?
- Sample files in the repo root: `Serato Recording 1.aif` (3 h, good), `2.aif` (80 min, good, not sequential with 1), `5.aif` (19 min, broken). All gitignored.
