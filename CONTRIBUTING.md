# Contributing

Thanks for helping. This is a small, free tool; the goal is that it never damages a recording and stays fast on multi-GB files.

## Before you start

- For anything bigger than a bug fix, open an issue first so we agree on the approach.
- Read [docs/FEATURES.md](docs/FEATURES.md) (scope) and [docs/ANALYSIS.md](docs/ANALYSIS.md) (decisions and why). [CLAUDE.md](CLAUDE.md) holds the engineering rules in full.
- Set up your machine with the [developer setup in the README](README.md#developer-setup). Windows 11 is the primary platform; macOS is verified by CI only.

## Workflow

1. Fork the repo and branch from `main`.
2. Make the change, with tests (see below).
3. Run `just ci` (or the equivalent commands in the README) and fix anything it reports.
4. Open a pull request and fill in the template. CI must be green before review.

## Rules that matter most

- **Never modify user recordings by default.** Repair writes a new file; in-place repair is an explicit choice that keeps a `.bak`.
- **Never touch a file Serato is still writing** (open handle or recent mtime). This has tests; keep them passing.
- **No silent truncation.** If AIFF output would exceed the 4 GB chunk limit, fail loudly. Never write a wrapped size.
- **Bounded memory.** Stream or memory-map; never read a multi-GB file into memory or send audio over Tauri IPC as JSON.
- **Core crates are pure.** No Tauri imports and no global state in `crates/*`, so they stay testable.
- **Paths are `Path`/`PathBuf`**, never string concatenation. Handle spaces and non-ASCII.
- **No new dependency without checking its license.** The project is GPL-3.0-or-later and `cargo deny` rejects anything incompatible. Mention new dependencies in the PR.

## Tests

A change is not done until the relevant layers pass:

| Change touches            | Run                                                                                               |
| ------------------------- | ------------------------------------------------------------------------------------------------- |
| `crates/*`                | `cargo test --workspace` (write the failing test first; fixtures are generated in code)           |
| `ui/src/lib/*.ts`         | `npm --prefix ui test`                                                                            |
| Svelte components         | the same, plus a component test against the fake backend in `ui/src/test/backend.ts`              |
| Anything user-visible     | `just e2e` (Windows; rebuild the exe first, or it tests the old app)                              |
| Any Rust                  | `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`             |
| Dependencies              | `cargo deny check`                                                                                |

Tests that need the real sample recordings are `#[ignore]` and optional: `just test-samples`. Do not commit sample `.aif` files; the small golden fixture in `tests/fixtures/` is the only one.

If a test fails, ask whether the app or the test is wrong. Both happen.

## Style

- `cargo fmt` and clippy clean, no warnings.
- Comments are sparse and explain why, not what. Match the surrounding code.
- UI changes stay inside the design tokens in `ui/src/app.css`: no rounded cards, no blue/teal accents, no system-ui fonts. Keep wording plain.

## Commits and pull requests

- Small, focused commits with a message that says why.
- User-visible changes get a line under "Unreleased" in [CHANGELOG.md](CHANGELOG.md). Dependabot PRs get theirs automatically (a maintainer can re-run it by commenting `/auto-changelog`).
- Docs-only PRs skip the build jobs; the required check is the single "CI result".
- One topic per PR. Include how you tested it and, for UI changes, a screenshot.
- Say which platform you tested on. Do not claim macOS behaviour works unless CI or a Mac confirmed it.

## Logging and privacy

- The app writes a local log (`src-tauri/src/logging.rs`) through the `log` crate: use `log::info!`/`warn!`/`error!` in `src-tauri`, and log **file names, not full paths**. Failures returned to the UI from a command should go through `logged(...)`.
- The app collects no data and that is a promise ([PRIVACY.md](PRIVACY.md)). Do not add telemetry, analytics or crash upload. Any new network access needs an opt-out, must send no identifiers, and must be described in `PRIVACY.md` in the same pull request. End-to-end tests set `RECORDING_FIXER_NO_UPDATE_CHECK` so they never touch the network.

## Security

Please do not file security problems as public issues. Use GitHub's private vulnerability reporting on the repository's Security tab.

## License

The project is [GPL-3.0-or-later](LICENSE). By submitting a contribution you agree it is licensed under the same terms. There is no CLA.
