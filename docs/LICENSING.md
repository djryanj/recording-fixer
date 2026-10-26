# Licensing

*Engineering notes, not legal advice.*

## Decision

**GPL-3.0-or-later.** The project is free, open source, and never to be sold. The [LICENSE](../LICENSE) file is the unmodified GPLv3 text from gnu.org; `Cargo.toml` and `ui/package.json` declare `GPL-3.0-or-later`.

Why this one: Steinberg's ASIO SDK, compiled into the app for ASIO output, is available under the GPLv3 (or a proprietary agreement). Choosing the GPL for the whole project means one build, with ASIO, and nothing to negotiate. The only obligation when handing a build to someone is to make the matching source available, which a public repository does.

## What is in the build

| Part | License | Notes |
|---|---|---|
| This project | GPL-3.0-or-later | |
| Rust crates (about 520: Tauri, cpal, rubato, ringbuf, libloading, dirs, ...) | MIT / Apache-2.0 / Zlib / ISC / BSD / Unicode, plus 5 MPL-2.0 (`cssparser`, `cssparser-macros`, `dtoa-short`, `selectors`, `option-ext`) | all GPLv3-compatible; MPL-2.0 only asks that changes to those files stay open |
| npm packages (Svelte, Vite, test tools; about 170, build and test time only except the fonts and Tauri API) | MIT, Apache-2.0, ISC, BSD, CC0, BlueOak, OFL; 12 are MPL-2.0 (Lightning CSS binaries pulled in by Vite 8, never shipped); one is `MIT OR GPL-3.0-or-later` (use MIT) | Svelte is MIT |
| Fonts: Barlow, Barlow Condensed, JetBrains Mono | SIL OFL 1.1 | keep the license text with any distributed fonts |
| Steinberg ASIO SDK (via cpal's `asio-sys`) | GPLv3 or Steinberg proprietary | used under the GPLv3; add the trademark line below |
| ffmpeg | separate program, run as a subprocess | LGPL build for any bundled copy; ffmpeg's license applies to that binary, not to this code |
| Apple `CoreAudioToolbox.dll` | Apple | loaded from the user's own install at runtime; never shipped or downloaded |
| VST3 SDK (stretch goal) | MIT since v3.8 | |

## Things to keep doing

- Keep the trademark notice wherever ASIO is mentioned: *"ASIO is a registered trademark of Steinberg Media Technologies GmbH."*
- The name uses "for Serato DJ" only to say what it works with, with a "not affiliated with Serato" statement in the README and in Settings > About (not elsewhere in the window). Do not use their logo.
- Releases bundle ffmpeg (an **LGPL** build, no `--enable-gpl`, no `--enable-nonfree`; see [THIRD-PARTY.md](THIRD-PARTY.md) and `scripts/fetch-ffmpeg.mjs`). Each release records the exact build and where its source is. Never `libfdk_aac`; never Apple's DLL.
- Release builds include ASIO by default (decided): the app is GPLv3, which is the licence the ASIO SDK is used under.
- If an outside contribution is ever accepted, it comes in under the same license.
- Optional hardening: `cargo deny check licenses` in CI would stop a non-GPL-compatible dependency from slipping in unnoticed.