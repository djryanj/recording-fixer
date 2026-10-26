## What and why

<!-- What changed, and the reason. Link the issue if there is one. -->

## How I tested it

<!-- Platform (Windows / macOS), which layers you ran, and anything you checked by hand. -->

- [ ] `just ci` passes (or: Rust tests, `npm --prefix ui test`, `cargo fmt`, clippy `-D warnings`)
- [ ] Core crate change: a failing test came first
- [ ] User-visible change: `just e2e` run on a freshly built exe, screenshot attached

## Checklist

- [ ] User-visible change: line added to `CHANGELOG.md` under Unreleased
- [ ] Does not modify user recordings by default, or touch a file Serato may still be writing
- [ ] No code path allocates in proportion to file size; no audio sent over IPC as JSON
- [ ] New dependencies are GPLv3-compatible (`cargo deny check` passes) and listed here: <!-- none -->
- [ ] I did not claim macOS behaviour I could not verify
- [ ] My contribution is under GPL-3.0-or-later
