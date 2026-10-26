# Changelog

All notable changes to this project are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/). The app version is `[workspace.package] version` in the root `Cargo.toml`.

## [Unreleased]

## [0.1.0] - Not yet released

First version, not yet tagged or published.

### Added

- Finds Serato DJ recordings (`.aif`) in `_Serato_/Recording` and `Recording temp`, including `_Serato_` folders on external drives, and lists them as the diagnosis runs.
- Diagnoses the broken-header bug (zeroed `FORM` size, `COMM` frame count and `SSND` size) from the header and file tail only, without reading the audio. Files still held open or recently modified are marked in use and never touched.
- Repairs broken files into a new file by default; in-place repair is an explicit choice and keeps a `.bak`.
- Non-destructive edit list: trim, split, remove and reorder clips, and combine several recordings, including ones with different sample rates. Nothing is rendered until export.
- Waveform timeline with rubber-band zoom down to single samples, drag to trim or reorder, and sample-accurate click, keyboard and `#N` positioning.
- Audio preview through WASAPI, ASIO (Windows build option) and CoreAudio, with device and output-pair selection.
- Export to AIFF, MP3 (CBR, VBR, ABR) and AAC through ffmpeg, with ID3 tags, filename and tag templates, and fades applied identically for every output.
- Apple AAC encoding on Windows through the user's own `CoreAudioToolbox.dll`, with encoder delay compensated. Encoder choice is detected at startup and can be overridden.
- Windows and macOS installers with ffmpeg included (an LGPL build), produced by a tag-triggered release workflow that tests the installed Windows app before drafting the release, with optional code signing (see docs/RELEASING.md). A user guide (docs/USER-GUIDE.md, linked from Settings > About) and a list of bundled components (docs/THIRD-PARTY.md).
- A loudness graph under the waveform (as recorded, and after processing, with the target), filled in progressively while a recording is measured for the first time. Measuring a 3 hour recording takes about 12 seconds (it took minutes in development builds before the audio crates were optimised there).
- Fades have their own tab in the editor, with quick presets, and a section in Settings.
- Hear the loudness processing in the editor preview (its own Loudness tab), with seeking, trimming and reordering all heard processed; the first measurement of a recording shows progress and is remembered.
- Waveforms now appear as they are built, with a visible progress mark, instead of after the whole file has been read.
- Loudness maximizer on export: match a target loudness (LUFS) or raise to a peak limit, with a leveller that follows the whole set (beatmixes down, quiet tracks up, breakdowns kept) and a true-peak lookahead limiter. Keeps the recording's bit depth, dithers 16-bit. Off by default. See [docs/MAXIMIZER.md](docs/MAXIMIZER.md).
- A local log file (errors, panics and what the app did) with Settings buttons to show it or copy it for a bug report. It is never sent anywhere.
- A private update check: one anonymous request to GitHub's public release list, at most daily, switchable off in Settings; it never downloads or installs anything. See [PRIVACY.md](PRIVACY.md): the app collects no data, ever.
- Settings dialog, Auto/Dark/Light theme, and an About page with licence and non-affiliation notices.
- CI for Windows and macOS (including the end-to-end suite against the real app on Windows, driven through msedgedriver), plus Dependabot and a `cargo deny` dependency policy. CI compiles no tools from source: the Tauri CLI is a UI dev dependency (`npm --prefix ui install`, used by the `just` recipes), and ffmpeg is a pinned, cached download.

### Known limitations

- macOS is covered by CI only and has not been run on a Mac.
- Real ASIO hardware (DJM-S7, DDJ-SX) is untested; ASIO4ALL and WASAPI were verified.
- Preview playback does not include fades.
- Installers, bundled ffmpeg and code signing are not set up yet.

[Unreleased]: https://github.com/djryanj/recording-fixer/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/djryanj/recording-fixer/releases/tag/v0.1.0
