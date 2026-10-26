# Third-party software in Recording Fixer for Serato DJ

This app is licensed under the [GNU GPL v3 or later](../LICENSE). It includes or uses the following. *Engineering notes, not legal advice.*

## ffmpeg (bundled, run as a separate program)

The installers include a copy of **ffmpeg**, which the app starts as a subprocess to make MP3 and AAC files. It is not linked into the app.

- **Licence: LGPL v2.1 or later.** The bundled builds are configured without `--enable-gpl` and `--enable-nonfree`. They contain no `libfdk_aac` and nothing from Apple's DLL. The licence text is at <https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html> and in the ffmpeg source.
- **Windows (x86_64):** the "lgpl" variant of the static builds from [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds), downloaded over HTTPS and checked against the checksum list published with them (`scripts/fetch-ffmpeg.mjs`). The exact file and its SHA-256 are recorded in `FFMPEG-BUILD.txt`, which sits next to the program in the install folder and is attached to each release. Source for FFmpeg: <https://ffmpeg.org/download.html>; the build scripts that produced the binary are in BtbN's repository.
- **macOS (universal: Apple silicon and Intel):** built from source for each release by `scripts/build-ffmpeg-macos.sh` in this repository: FFmpeg 7.1.1 (<https://ffmpeg.org/releases/ffmpeg-7.1.1.tar.xz>) with LAME 3.100 (LGPL, <https://sourceforge.net/projects/lame/files/lame/3.100/>) for MP3, and Apple's AudioToolbox (part of macOS) for AAC. Versions, checksums and the configuration are in `FFMPEG-BUILD.txt`.
- **Replacing it:** because ffmpeg is a separate program, you can use a different build: Settings → ffmpeg → *Choose a different ffmpeg…*.

If you want the complete corresponding source of the ffmpeg in a release you downloaded and cannot get it from the links above, open an issue and it will be provided.

## ASIO (Windows)

The Windows build includes ASIO output through Steinberg's ASIO SDK (pulled in by the `cpal` audio library). The SDK is available under the GPLv3, which is the licence this app uses it under. *ASIO is a registered trademark of Steinberg Media Technologies GmbH.*

## Apple's AAC encoder

Not included. On macOS the app uses the encoder that is part of the operating system (through AudioToolbox). On Windows it uses `CoreAudioToolbox.dll` from the user's own Apple Application Support / iTunes installation if there is one, loaded at run time. It is never shipped or downloaded by this app.

## Microsoft Edge WebView2 (Windows)

The window is drawn by the system's WebView2 runtime; the installer sets it up if it is missing. It is Microsoft's component under Microsoft's terms and is not modified.

## Libraries compiled in

About 520 Rust crates (the Tauri application framework, `cpal`, `rubato`, `ringbuf`, `libloading`, `serde` and others), under MIT, Apache-2.0, ISC, BSD, Zlib, Unicode and CC0 licences, plus a few under MPL-2.0 (`cssparser`, `selectors` and related). All are compatible with the GPLv3. The full list with versions is `Cargo.lock`; every build is checked by `cargo deny` against an allow-list of licences (see `deny.toml`).

## Fonts

*Barlow*, *Barlow Condensed* and *JetBrains Mono*, under the SIL Open Font License 1.1, bundled through the `@fontsource` packages.

## Not affiliated

"Serato" and "Serato DJ" are trademarks of their owner. This project is not affiliated with or endorsed by Serato.
