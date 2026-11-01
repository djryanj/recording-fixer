# Privacy policy

*Recording Fixer for Serato DJ. Last updated 2026-10-26.*

**This app does not collect any data. Not now, and not ever.** There is no telemetry, no analytics, no crash reporting, no account, no advertising and no tracking of any kind, and none will be added. If that ever had to change, it would be a new, clearly announced version with an opt-in, never a quiet update, and this document would change with it (its history is public in this repository).

## Your recordings and files

Everything the app does with your recordings (scanning, repairing, editing, playing, exporting) happens on your computer. The app never uploads, shares or transmits your audio, your file names, your folders or your settings.

## The one thing that goes online: the update check

When the app starts (at most once a day), and when you press **Check now** in Settings, it asks whether a newer version exists. To do that it makes one HTTPS request to GitHub's public API for this project's latest release:

`https://api.github.com/repos/djryanj/recording-fixer/releases/latest`

- The request carries **no identifier of any kind**: no version number, operating system, user name, machine ID or cookie. Its only header content beyond what any web request needs is a fixed text, `recording-fixer-update-check`, which GitHub requires.
- Like any web request, it reveals your **IP address to GitHub** (which hosts this project), and GitHub may log requests under its own [privacy statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement). The app has no access to those logs and sends GitHub nothing beyond the request itself.
- The reply is a public release description. The app compares version numbers on your computer. It **never downloads or installs anything by itself**; if a newer version exists it shows a notice with a link, and the download happens in your browser only if you click it.
- The only thing remembered is *when* the last check happened, on your computer, so it does not ask more than once a day.
- You can switch the automatic check off in **Settings > Updates and privacy**. With it off, the app makes no network requests at all.

The request is made by `curl`, which comes with Windows 10 and later and with macOS. If it is missing, the check quietly does nothing.

## What stays on your computer

- **A log file** of what the app did and any errors, to help with bug reports. It is written only on your computer and is never sent anywhere. It can contain the **names** of recordings you worked with and error messages from your system, so look through it before you share it. It is kept small (two files of up to 1 MB each, the oldest overwritten). **Settings > Troubleshooting > Show log file** opens its folder; deleting the files is always safe.
- **Your settings** (theme, default export options, chosen folders, ffmpeg location), stored by the app's built-in web view in your user profile.
- **A waveform cache** of the recordings you opened, limited to 300 MB and rebuilt on demand, in the app's cache folder.
- **Files you create** (repaired copies, exports) in the folders you choose. Nothing else is written next to your recordings; the originals are not modified unless you pick in-place repair, which keeps a backup.

Uninstalling the app and deleting its folders removes all of this.

## Other software involved

- **ffmpeg**, which the app runs locally to make MP3 and AAC files, is a separate program. The app gives it audio and file names on your computer only.
- The window is drawn by your system's web view (Microsoft Edge WebView2 on Windows, WebKit on macOS). The app loads no web pages, fonts or scripts from the internet. Those components are Microsoft's and Apple's and have their own policies.
- **Links** (Report an issue, Source code, this policy, a release page) open in your normal browser, only when you click them, and then you are on GitHub under GitHub's terms.

## Questions

Open an issue at <https://github.com/djryanj/recording-fixer/issues>. Please do not post log files or recordings publicly without reading them first.
