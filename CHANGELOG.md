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
- Settings dialog, Auto/Dark/Light theme, and an About page with licence and non-affiliation notices.
- CI for Windows and macOS, plus Dependabot and a `cargo deny` dependency policy.

### Known limitations

- macOS is covered by CI only and has not been run on a Mac.
- Real ASIO hardware (DJM-S7, DDJ-SX) is untested; ASIO4ALL and WASAPI were verified.
- Preview playback does not include fades.
- Installers, bundled ffmpeg and code signing are not set up yet.

[Unreleased]: https://github.com/djryanj/recording-fixer/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/djryanj/recording-fixer/releases/tag/v0.1.0
