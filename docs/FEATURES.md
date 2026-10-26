# Feature list

Priority: **P0** = MVP, **P1** = v1.0, **P2** = later, **S** = stretch.

## MVP status (as of the first build)

**Done and tested:** folder discovery and persistence; unified list with status, duration, format, recorded time (ID3), sort/filter/search; in-use detection (Windows write-lock, plus recency for unclosed files); repair to a copy or in place with header backup, cancel and progress; combine with reorder and trim; export to AIFF, MP3 and AAC via ffmpeg, streaming with no temp files; ffmpeg discovery (user path, beside the exe, PATH); AAC encoder detection and preference order; Apple AAC *detection*; 4 GB AIFF limit guard; refresh on window focus.

**Added in the second pass:** waveform timeline (streaming multi-resolution peaks, disk cache, zoom/scroll, drag-trim, drag-reorder, split at cursor, remove, trim-to-cursor, keyboard shortcuts, per-clip time fields); one file per clip (split a set into tracks); Apple AAC encoding on Windows through `CoreAudioToolbox.dll` (CBR and true VBR); MP3 CBR/VBR/ABR with ID3v2.3/2.4 and ID3v1; AAC CBR/VBR per encoder; filename templates with placeholders; full tag editing (title, artist, album, album artist, genre, date, track, comment) with placeholders; size estimate; remembered export settings.

**Added in the third pass:** in-app playback of recordings and of the edit as you change it (play/pause/seek, level meters, volume); audio output chooser with ASIO, WASAPI and CoreAudio devices, output channel pair and buffer size, remembered across runs; resampling when the device rate differs; fade in / fade out on export (all formats, shown on the timeline); Apple AAC encoder delay compensated; a new look (instrument-panel design, Auto/Dark/Light themes, self-hosted fonts); an automated test pyramid including real-app end-to-end tests.

**Not done yet (from the P0 list):** bundling an LGPL ffmpeg sidecar; Windows/macOS CI; scan progress streaming (the scan is one call, fast because it reads headers only); external-drive `_Serato_` discovery. The macOS path is untested on a Mac.

**Not done (P1+):** fades in the playback preview, crossfades between clips, loop regions, device-latency compensation of the playhead, resampling for mixed rates, Apple AAC on macOS outside ffmpeg's `aac_at`, per-recording cover art, loudness normalisation, spectrogram, VST3.

Legend: items below are the full plan, not what is built.

## Cross-cutting: performance and memory (applies to every feature)
- **P0** Memory stays bounded regardless of file size (streaming I/O, no whole-file buffers); target < 150 MB resident on a 3 GB file.
- **P0** Scan and diagnose read only headers and tails, never audio; list populates progressively.
- **P0** All heavy work off the UI thread, with progress and cancellation.
- **P0** Repair and same-format combine run at near disk speed.
- **P1** Bounded parallelism for scans and peak builds; tuned for external/slow drives.
- **P1** Size-capped peak cache with eviction; waveform overview of a 3 h file in seconds, 60 fps zoom/scroll.
- **P1** Benchmarks and a peak-RSS regression test in CI; baselines tracked in `docs/PERF.md`.
- Details and rules are in CLAUDE.md under "Performance and memory requirements".

## Library (list-first home screen)
- **P0** Auto-discover Serato recording folders (`Recording`, `Recording temp`) on Windows and macOS.
- **P0** User can add/remove/choose folders; choices persist.
- **P0** Unified list across all folders: name, folder, duration, size, format (rate/bit depth/channels), modified date, **status**.
- **P0** Status per file: `OK` / `Broken header` / `In use (Serato running?)` / `Unreadable`.
- **P0** Read the trailing ID3 tag (title, `Recorded <date/time>` comment) for display and sort-by-recorded-time.
- **P0** `Recording temp` files are treated as in-progress/orphaned: flagged "in use" unless closed and old.
- **P0** Sort, filter by status/folder, multi-select.
- **P1** Scan external-drive `_Serato_` folders.
- **P1** Quick preview playback from the list (click to audition).
- **P1** Inline mini-waveform in each row.
- **P2** Rename; reveal in Explorer/Finder; delete to trash.

## Repair
- **P0** Detect zeroed/incorrect `FORM`, `COMM.numSampleFrames`, `SSND` sizes.
- **P0** Repair to a new file (default) or in place with `.bak`.
- **P0** Batch repair of all selected broken files.
- **P0** Trim a partial trailing frame; report what was changed.
- **P0** Refuse to touch files that are currently being written.
- **P1** Dry-run report (what would change) before applying.
- **P1** Handle 24-bit, mono, AIFC variants.
- **P2** Verify repaired file via ffprobe and show the result.

## Combine
- **P0** Select multiple files from any folders and combine them in order.
- **P0** Non-destructive edit list: reorder, trim head/tail of each clip. Nothing is written until export.
- **P1** Visual waveform timeline of clips, drag to reorder, drag edges to trim, zoom/scroll.
- **P1** Cut out a region inside a clip (split + delete); gap and join markers.
- **P1** Crossfade or short fade at joins; optional gain per clip.
- **P1** Save/load the edit list as a project file.
- **P1** Auto-suggest joining likely continuations (same session, consecutive timestamps, 3 h splits).
- **P1** Handle mixed sample rate/bit depth by resampling via ffmpeg.
- **P1** Warn / offer alternatives when AIFF output would exceed 4 GB.
- **P2** Detect and optionally trim silence at the start and end.

## Export
- **P0** Export to mp3 or aac (m4a) via ffmpeg; bitrate/quality presets (e.g. 192/256/320 kbps; VBR).
- **P0** Export to AIFF (lossless) for repaired/combined output.
- **P0** Locate ffmpeg: bundled LGPL build by default; user-supplied binary selectable (e.g. with libfdk_aac).
- **P0** Detect available AAC encoders (`ffmpeg -encoders`) and use the best; display which one is active.
- **P1** Detect and use Apple AAC: macOS `aac_at`/`afconvert`; Windows via `CoreAudioToolbox.dll` if Apple Application Support is installed (never redistributed). Behaviour modelled on OBS.
- **P0** Progress, cancel, and error reporting.
- **P1** Metadata: title/artist/date tags, auto-title from recording timestamp.
- **P1** Output naming templates and destination folder choice.
- **P1** Optional FLAC/WAV output.
- **P1** Loudness normalise (ffmpeg `loudnorm`) option.
- **P2** Split a long recording into tracks (silence detection or manual markers), exported as separate files.

## Waveform editor
- **P1** Peak cache (multi-resolution) built in the background, reused across sessions.
- **P1** Smooth zoom and scroll on multi-hour files.
- **P1** Play / seek / loop selection.
- **P2** Spectrogram toggle.

## App
- **P0** Windows and macOS builds in CI; unsigned installers to start.
- **P1** Light/dark theme following the OS.
- **P1** Drag-and-drop files and folders into the app.
- **P2** Code-signed/notarised builds, auto-update.

## Stretch: VST3
- **S** Plugin scan and a per-project plugin chain (isolated host process).
- **S** Apply the chain on export (offline render).
- **S** Realtime preview through the chain.
- **S** Plugin state saved with the project.

## Explicit non-goals (for now)
- Multitrack mixing, MIDI, or general DAW functions.
- Editing Serato's own library/crates database.
- Cloud features.
