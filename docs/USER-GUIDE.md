# Recording Fixer for Serato DJ: user guide

Recording Fixer finds the recordings Serato DJ made, fixes the ones that Serato never closed properly, lets you trim and join them, and saves the result as AIFF, MP3 or AAC. It is free, open source, and collects nothing about you (see [Privacy](#privacy-and-updates)).

It is not affiliated with or endorsed by Serato.

Contents: [Installing](#installing) · [The first minute](#the-first-minute) · [Repairing](#repairing-a-recording) · [The editor](#the-editor) · [Exporting](#exporting) · [Loudness](#loudness) · [Fades](#fades) · [Playing and audio output](#playing-and-audio-output) · [Settings](#settings) · [Problems](#when-something-goes-wrong) · [Questions](#questions)

---

## Installing

**Windows 10 or 11 (64-bit).** Run the `Recording Fixer for Serato DJ_…_x64-setup.exe` from the [Releases page](https://github.com/djryanj/recording-fixer/releases). It installs for your user only (no administrator rights) and adds a Start menu entry. The installer sets up Microsoft's WebView2 if your PC does not have it yet.

**macOS 11 or newer (Apple silicon and Intel).** Open the `.dmg` and drag the app into Applications.

**ffmpeg is included.** MP3 and AAC export use ffmpeg, and a copy comes with the app, so there is nothing else to install.

### "Windows protected your PC" / "can't be opened because Apple cannot check it"

Early builds may not be signed with a publisher certificate yet. That makes the system show a warning for any program it has not seen many times; it does not mean the file is damaged. To check you have the real file, compare its SHA-256 with `SHA256SUMS.txt` on the release page:

- Windows: in PowerShell, `Get-FileHash` on the downloaded file, for example `Get-FileHash "$HOME\Downloads\Recording Fixer for Serato DJ_0.1.0_x64-setup.exe"`
- macOS: in Terminal, `shasum -a 256 Recording-Fixer….dmg`

Then:

- **Windows:** in the blue window choose **More info**, then **Run anyway**.
- **macOS:** Control-click (right-click) the app, choose **Open**, then **Open** again. Or: System Settings → Privacy & Security → scroll down → **Open Anyway**.

You only need to do this once.

### Updating and uninstalling

The app can tell you when a newer version exists (Settings → Updates and privacy) but never installs anything by itself: download the new installer and run it over the old one. To uninstall use Windows' Apps list, or drag the app to the Bin on macOS. Your recordings are never touched by either.

---

## The first minute

Start the app. It lists the recordings in Serato's folders, newest first:

- `Music\_Serato_\Recording`: sets Serato saved when you stopped recording normally.
- `Music\_Serato_\Recording temp`: recordings Serato was still writing. If Serato closed or crashed first, they stay here and usually need repair.
- A `_Serato_` folder on an external drive, if there is one.

Use **+ Add folder…** for anywhere else, and tick or untick the folders under **Show recordings from**. The list updates when you come back to the window, or press **Rescan**.

Each row shows a **status**:

| Status | Meaning | What you can do |
|---|---|---|
| **OK** | The file is fine. | Play, edit, export. |
| **Needs repair** | Serato never wrote the length into the file header. The audio is all there. | Repair it, or export it straight away (the editor can read it as it is). |
| **In use** | Serato may still be recording it (it is open, or was just changed). | Nothing. The app will never touch a file Serato might be writing. |
| **Unreadable / Unsupported** | Not an AIFF file the app understands. | Nothing. |

Click ▶ on a row to listen to the whole file. **Space** plays and pauses.

---

## Repairing a recording

Select the file(s) marked **Needs repair** and press **Repair…**. You choose:

- **Save a repaired copy** (recommended). A new file `name (fixed).aif` appears next to the original, which is left exactly as it was.
- **Fix in place.** Quicker and needs no extra disk space. The original header is first saved as `name.aif.hdrbak`, so it can be undone.

The repair only writes three numbers into the file's header; it does not re-encode anything, so it is lossless and takes a moment even for a 3 GB set.

You do not have to repair a file before editing or exporting it.

---

## The editor

Select one or more recordings and press **Edit & export…**. The editor shows every selected recording as a **clip** on a waveform timeline, in the order they were recorded. Nothing is changed in your files: the editor only keeps a list of what to include, and nothing is written until you export.

### Moving around

- **Click** the waveform (or the ruler) to play from there. **Space** pauses and resumes.
- **Drag across** a waveform to zoom to that stretch. **Ctrl + mouse wheel** zooms at the pointer, the wheel scrolls, **Fit** shows everything. You can zoom right down to single samples.
- Arrow keys move the playhead (hold **Shift** for 10 seconds, **Alt** for 0.1 s).

### Cutting

| To… | Do this |
|---|---|
| **Trim** the start or end of a clip | Drag its edge, or **Trim start here** / **Trim end here** at the playhead. |
| **Split** a clip in two | Put the playhead where you want the cut and press **Split at playhead** (`S`). |
| **Cut a section out** | Split at the start, split at the end, then select the middle piece and **Remove** it (`Delete`). |
| **Reorder** clips | Drag the clip by its name bar at the top. |
| Type an exact position | Edit the time fields, or enter `#12345` for sample number 12345. |

What you hear in the editor is what you have cut.

### One file or many

Choose **Combine into one file**, or **One file per clip**, which is handy for turning a long set into separate tracks.

Clips you combine must have the same sample rate, bit depth and number of channels (resampling is not built yet). The editor tells you if they do not match.

---

## Exporting

Press **Export**. The settings are in tabs on the right of the editor, and the app remembers what you chose last time.

### Format & quality

| Format | Good for | Notes |
|---|---|---|
| **AIFF** | An exact, lossless copy; further editing. | Large. A single AIFF cannot be bigger than 4 GB (about 6 hours of CD-quality stereo); the app warns you. No tags. |
| **MP3** | Sharing and phones; works everywhere. | **Constant** (CBR, 320 kbps is the safe choice), **Variable** (VBR, V0 is the best quality for its size) or **Average** (ABR). ID3 tags version 2.3 (most compatible) or 2.4, optionally also ID3v1. |
| **AAC** (.m4a) | Same quality as MP3 in less space; Apple devices. | Constant or variable quality. On a Mac the encoder is Apple's own; on Windows see [Apple's AAC encoder](#apples-aac-encoder-windows-optional). |

The estimated size is shown under the options.

### Loudness and fades

Two more tabs: [Loudness](#loudness) and [Fades](#fades), described below.

### Name, folder & tags

- **File name**: a template such as `{date} {time} - {name}`. The placeholders are `{name}` `{date}` `{time}` `{clock}` `{datetime}` `{year}` `{n}` `{n2}` `{count}` `{ext}` `{quality}`. Date and time come from the recording itself, not from today. The preview below shows the result for each file. The same placeholders work in the tag fields.
- **Folder** to save into (it defaults to the recording's folder). Existing files are never overwritten: a number is added to the name instead.
- **Tags** (MP3 and AAC): title, artist, album, album artist, genre, date, track and comment.

Progress is shown at the bottom of the window, with **Cancel**. A cancelled or failed export leaves no half-written file behind. When it finishes there is a **Show** button that opens the folder.

---

## Loudness

DJ sets recorded from Serato are often quiet, and uneven: a beatmix, where two tracks play at once, is louder than a single track, and now and then one track was simply played at a lower level than the rest. The **Loudness** tab fixes that on export, as part of the same step as the conversion. It is **off** unless you choose it, and when off the audio is exported exactly as recorded.

### The choices

- **Leave as recorded**: no change.
- **Match a loudness**: brings the whole set to a target loudness. **−14 LUFS** is the usual level for streaming services; **−16** is quieter, **−11** louder. (LUFS is the standard unit of perceived loudness; more negative is quieter.)
- **Raise to a peak limit**: one steady change in volume so that the highest peak just reaches the limit (for example −0.1 dB). Nothing else is altered.

For *Match a loudness* you also choose:

- **Evening out**: *None* applies one volume change to the whole set. *Gentle*, *Medium* (recommended) and *Strong* also turn down the stretches that are much louder than their surroundings and turn up the stretches that are much quieter, by a fraction of the difference. A breakdown that is quiet on purpose is left alone, and so is the slow build of a set, so the shape of your set survives.
- **Peak limit** (a "true peak" in dBTP): nothing will go above this. A limiter catches the occasional peak that would otherwise exceed it. **−1 dBTP** is a good value, especially for MP3 and AAC, which can overshoot a little when they are encoded.

The result keeps the recording's own bit depth: 16-bit stays 16-bit (with a tiny amount of dither added, which is the standard way to do this cleanly).

### Seeing and hearing it first

With the Loudness tab open, a **graph appears under the waveform**: the loudness over time as recorded (grey) and, once you choose a mode, what it will be afterwards (amber), with the target as a dashed line. You can see the beatmixes that stick out and the tracks that are too quiet.

And you can **hear it**: with *Hear this when playing in the editor* ticked, playing (and seeking anywhere) in the editor is processed the same way as the export will be. Untick it to compare with the original. It is accurate to about half a LU; the export makes one more check pass to land exactly on the target.

The first time a recording is used, it has to be measured from start to end: about 12 seconds for a three-hour set. The graph fills in as that happens, a progress bar shows how far it has got, and the original keeps playing until it finishes. It is remembered afterwards, even when you restart.

When an export finishes, the message under the file name says what happened, for example *"Loudness −14.0 LUFS (was −19.6), peak −1.0 dBTP; the limiter averaged 0.0 dB and reached 4.4 dB at most."*

### A word on how far to push it

A set that is already loud has little room. If the message shows the limiter averaging more than about 1–2 dB, the target is high for this material and it will start to sound squashed: choose a lower target. Recordings that have already been clipped by Serato cannot be repaired by any limiter.

---

## Fades

The **Fades** tab adds a fade in at the start and a fade out at the end of each exported file, in seconds, for every format. A set that stops abruptly (or starts mid-track) sounds finished with a few seconds of fade. The timeline shows the fades as dimmed ramps over the waveform. Fades are applied on export; they are not played in the editor.

---

## Playing and audio output

**Output** (in the bar at the bottom) chooses where the sound goes. Devices are grouped by how they talk to the system:

- **ASIO**: low-delay drivers, which is what DJ interfaces such as the Pioneer DJM-S7 and DDJ-SX provide. You can pick which pair of channels to play on (for example 3–4 for a booth output) and a buffer size. Most ASIO drivers serve only one program at a time, so if a device will not open, close Serato (or choose another output); the app shows the driver's own message.
- **WASAPI** (Windows) or **CoreAudio** (macOS): the normal system output.

Your choice is remembered. If that device is gone next time, the app uses the system default. Recordings are resampled while playing if the device wants a different rate; exports are never resampled.

### Apple's AAC encoder (Windows, optional)

On a Mac, AAC uses Apple's high-quality encoder automatically. On Windows it is available if you have **iTunes** installed (Apple does not offer the encoder on its own): the app finds it there and uses it by itself. You can see what it found under Settings → ffmpeg → AAC. Without it, a good built-in encoder is used instead.

---

## Settings

The **⚙ Settings** button holds:

- **Default export**: the format, quality, fades and loudness that a new export starts with.
- **ffmpeg**: shows which ffmpeg is used and where it was found. The one included with the app is used unless you point the app at another with **Choose a different ffmpeg…** (for example one with extra codecs).
- **Playback**: opens the audio output window.
- **Updates and privacy**: whether the app looks for a newer version at start-up, and a **Check now** button.
- **Troubleshooting**: **Show log file** and **Copy log to clipboard**.
- **About**: the version, licence and a link to report a problem.

The colour theme (Auto, Dark or Light) is the button next to Settings.

---

## Privacy and updates

The app collects nothing, ever: no usage statistics, no crash reports, no account. Your recordings and their names never leave your computer. The full statement is in [PRIVACY.md](../PRIVACY.md).

The one thing it can do online is ask GitHub whether a newer version exists: a plain web request with no identifier, at most once a day, which you can turn off in Settings. It never downloads or installs anything on its own.

---

## When something goes wrong

**The app keeps a log** of what it did and any errors, on your computer only. Settings → Troubleshooting → **Show log file** opens its folder; **Copy log to clipboard** copies the end of it so you can paste it into a bug report. It can contain the names of the recordings you worked with, so have a look before you share it. (On Windows the folder is `%LOCALAPPDATA%\com.ryanjjacobs.recording-fixer\logs`.)

**Report a problem** with the button in Settings → About (or the link at the bottom of the window): [github.com/djryanj/recording-fixer/issues](https://github.com/djryanj/recording-fixer/issues). Say what you did, what you expected, and what happened, and attach the log.

| Problem | Try |
|---|---|
| A file says **In use** | Close Serato (or wait: it was changed in the last few minutes). The app will not touch it until Serato is done. |
| A recording is missing from the list | Is its folder ticked under **Show recordings from**? Add the folder with **+ Add folder…**, then **Rescan**. |
| MP3 or AAC is greyed out | Open Settings → ffmpeg. The included ffmpeg should be found automatically; if you pointed the app at another one, **Use the automatic one**. |
| The output device will not open | Close Serato or other audio programs that hold the device (especially ASIO), then choose it again in **Output**. |
| No sound | Check **Output** shows the device you expect and that the volume slider is up. |
| Export says it is over 4 GB | AIFF cannot hold that much. Choose MP3 or AAC, or export one file per clip. |
| The first waveform of a very long file takes a while | It is built once, from the start, and you can see it grow. Afterwards it is cached. |
| The window is blank | Please report it with the log. Restarting the app often clears it. |

---

## Questions

**Does it change my original recordings?** No. Repair makes a copy by default (and in-place repair keeps a backup of the header). Editing is a list of instructions until you export, and exports are always new files.

**Is anything uploaded?** No. See [Privacy](#privacy-and-updates).

**Why does a repaired file open everywhere now?** Serato writes placeholders for the file's length at the start of a recording and fills them in when you stop. If Serato exits first, the audio is complete but the placeholders stay at zero and players see an empty file. The repair fills in the three numbers; see [ANALYSIS.md](ANALYSIS.md) for the details.

**Can it use VST plug-ins?** Not yet. The loudness tab covers the usual "make the set loud and even" job without one.

**Which licence is it under?** GNU GPL v3 or later. See [LICENSE](../LICENSE) and [THIRD-PARTY.md](THIRD-PARTY.md) for the components included, such as ffmpeg.
