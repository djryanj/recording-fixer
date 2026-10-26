# Releasing

How a release is made, how it is tested, and how to get it **signed**. For the maintainer.

## The steps

1. `just prepare-release 0.1.0-rc.1` on a clean `main`: makes `release/v0.1.0-rc.1` with the versions bumped, pushed. (A final release also dates the changelog.)
2. Open the PR, let CI pass, merge.
3. `git checkout main && git pull`, then `just tag-release 0.1.0-rc.1`: tags `main` and pushes the tag.
4. The **Release** workflow (`.github/workflows/release.yml`) starts from the tag:
   - **verify**: the tag equals the app version (`Cargo.toml` = `ui/package.json`) and is on `main`.
   - **build** (Windows and macOS in parallel): fetch or build the bundled **ffmpeg**, build the installer, sign it if signing is set up, gather the files with SHA-256 sums.
   - **Windows only: the installer is installed and the end-to-end suite is run against the installed app**, with `E2E_EXPECT_BUNDLED_FFMPEG` set so it also checks that MP3/AAC use the ffmpeg inside the install folder. This is the release build, not a debug build, so a problem that only exists in the shipped binary fails the release before anything is published. (macOS has no WebDriver; its build is covered by the component tests and CI.)
   - **publish**: only if both builds pass, a **draft** GitHub release is created with the installers, `SHA256SUMS.txt`, the ffmpeg build records and notes taken from the changelog. A tag containing `-` (such as `-rc.1`) is marked as a pre-release.
5. Open the draft, download and try the installers (ideally on machines that are not yours), edit the notes, and **publish**. Nothing is public until you do.

**Trial run:** Actions → *Release* → *Run workflow* builds and tests everything and keeps the files as workflow artifacts without releasing anything. Do this after changing the workflow or the ffmpeg scripts.

If a step fails after the tag was pushed, fix the problem on `main` and release the next version; do not move a tag that has built anything.

## The bundled ffmpeg

`src-tauri/tauri.release.conf.json` (used only by release builds) adds ffmpeg as a Tauri sidecar and the licence files as resources. Development and CI builds do not use it, so contributors do not need the binary.

| Platform | Where it comes from |
|---|---|
| Windows | BtbN's LGPL build, downloaded and checked against the checksum list published with it (`scripts/fetch-ffmpeg.mjs`). The release is called "latest", so this is an integrity check, not a pin: each release records the exact file and hash in `FFMPEG-BUILD.txt`. For a fully reproducible pin, host a chosen build as an asset of this repository and point the script at it. |
| macOS | Built from source by `scripts/build-ffmpeg-macos.sh`: FFmpeg 7.1.1 and LAME 3.100, pinned by SHA-256, LGPL only, AudioToolbox for AAC. Takes about 10 minutes on the runner. **This script has not yet been run on a Mac**; the trial run is how to find out. |

To change the ffmpeg version, edit the pinned values (the archive name in `fetch-ffmpeg.mjs`; the versions and hashes at the top of `build-ffmpeg-macos.sh`), run a trial, and update `docs/THIRD-PARTY.md`.

## Signing

Unsigned installers work, but Windows SmartScreen and macOS Gatekeeper warn about them (the user guide explains the click-through). Signing needs a certificate that **only you can obtain**, because it is tied to an identity. The workflow already contains the signing steps; they switch on when the settings below exist, so you can add them at any time without changing the workflow.

### Windows: SignPath Foundation (free for open source)

[SignPath Foundation](https://signpath.org/) offers free code signing for open-source projects, with a certificate issued in the foundation's name, and integrates with GitHub Actions. The terms and what they ask of a project change, so read them when you apply.

1. Apply at <https://signpath.org/> as an open-source project (GPL-3.0-or-later, public repository). Expect some back-and-forth and a wait.
2. In SignPath, create the project, an *artifact configuration* that signs a single `.exe` (the NSIS installer), and a *signing policy* (for example `release-signing`). Connect it to this GitHub repository (the "trusted build system" setting), so only builds made by this workflow can request signatures.
3. In the repository's **Settings → Secrets and variables → Actions**:
   - *Secret* `SIGNPATH_API_TOKEN`
   - *Variables* `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`
4. Run a trial. The workflow uploads the unsigned installer, waits for SignPath (it may need approving in SignPath's web page, depending on the policy), and carries on with the signed file. The signed file is the one that is installed for the end-to-end tests and published.

Alternatives if SignPath does not work out: **Azure Artifact Signing** (formerly Trusted Signing; paid, around ten dollars a month, identity checks), or a conventional OV/EV code-signing certificate on an HSM or cloud signer. Either needs a different signing step in `release.yml` in place of the SignPath ones. Tauri also supports a custom `bundle.windows.signCommand`.

Check a signed installer: `Get-AuthenticodeSignature .\…-setup.exe | Format-List` (Status should be `Valid`), or right-click → Properties → Digital Signatures.

### macOS: Apple Developer ID and notarization

There is no free route. You need the **Apple Developer Program** (99 USD a year), then:

1. In your developer account create a **Developer ID Application** certificate (Certificates, IDs & Profiles), download it, import it into Keychain Access on a Mac, and export it with its private key as a `.p12` with a password.
2. `base64 -i certificate.p12 | pbcopy` gives the value for the secret below.
3. Create an **app-specific password** at <https://appleid.apple.com/> (Sign-In and Security) for notarization. Note your **Team ID** (developer account → Membership).
4. Repository **Settings → Secrets and variables → Actions → Secrets**:

   | Secret | Value |
   |---|---|
   | `APPLE_CERTIFICATE` | the base64 text of the `.p12` |
   | `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password |
   | `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
   | `APPLE_ID` | your Apple ID e-mail |
   | `APPLE_APP_SPECIFIC_PASSWORD` | the app-specific password |
   | `APPLE_TEAM_ID` | the Team ID |

5. Run a trial. With `APPLE_CERTIFICATE` set, the macOS build step that signs and notarizes is used instead of the unsigned one (Tauri signs the app and the bundled ffmpeg, submits to Apple's notary service and staples the result).

Check it on a Mac: `codesign -dv --verbose=4 "Recording Fixer for Serato DJ.app"`, `spctl -a -vv "Recording Fixer for Serato DJ.app"` (should say *accepted, source=Notarized Developer ID*), `xcrun stapler validate` on the dmg.

### Keep the secrets safe

The signing steps run only in this workflow, from a tag on `main`; the workflow uses pinned actions; secrets are passed to single steps and never printed. Protect the `v*` tags in the repository rules so only maintainers can create them, otherwise anyone who can push a tag can ask for a signature.

## What is not covered yet

- A macOS end-to-end run (no WebDriver for WKWebView); only the build and the component tests.
- A Linux build.
- Automatic updates: the app only tells the user a newer version exists (see PRIVACY.md).
