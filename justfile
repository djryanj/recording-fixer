# Developer shortcuts. Install `just` first: https://github.com/casey/just
#   Windows: winget install --id Casey.Just -e     macOS: brew install just
# Run `just` with no arguments to list the recipes.

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# ASIO (Windows) needs LLVM's libclang; macOS builds leave ASIO out.
export LIBCLANG_PATH := env("LIBCLANG_PATH", if os() == "windows" { "C:\\Program Files\\LLVM\\bin" } else { "" })
features := if os() == "macos" { "--no-default-features" } else { "" }
# The Tauri CLI is a UI dev dependency (`just setup`); npm runs it from the repo root, where it finds src-tauri.
tauri := "npm --prefix ui exec -- tauri"

default:
    @just --list

# Install UI dependencies (run once, and after package.json changes)
setup:
    npm --prefix ui install

# Run the app with hot reload
dev:
    {{ tauri }} dev {{ if os() == "macos" { "-- --no-default-features" } else { "" } }}

# Format, lints and type check; must be clean
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets {{ features }} -- -D warnings
    npm --prefix ui run check

# Apply formatting
fmt:
    cargo fmt --all

# Rust and UI unit/component tests, and the release script's
test:
    cargo test --workspace {{ features }}
    npm --prefix ui test
    node --test "scripts/*.test.mjs"

# Also run the tests that need the real sample files in the repo root
test-samples:
    cargo test --workspace {{ features }} -- --include-ignored

# Build the debug app and drive it end to end (Windows only; see README)
e2e: build-debug
    npm --prefix ui run e2e

# Start the debug exe through msedgedriver with its verbose log (for e2e session failures)
e2e-diagnose:
    node ui/e2e/diagnose.mjs

# Quick debug exe, no installer
build-debug:
    {{ tauri }} build --debug --no-bundle {{ if os() == "macos" { "-- --no-default-features" } else { "" } }}

# Optimised standalone app for this machine, no installer: target/release/recording-fixer(.exe)
release:
    {{ tauri }} build --no-bundle {{ if os() == "macos" { "-- --no-default-features" } else { "" } }}

# Installers (NSIS/MSI on Windows, .dmg on macOS)
build:
    {{ tauri }} build {{ if os() == "macos" { "-- --no-default-features" } else { "" } }}

# Start a release: a release/vVERSION branch from a clean main with the versions bumped (and, for a
# final release, the changelog closed out), pushed. Then open a PR. e.g. just prepare-release 0.1.0-rc.1
prepare-release version:
    node scripts/release.mjs prepare {{ version }}

# After that PR is merged: tag main as vVERSION and push the tag. This is the last step.
tag-release version:
    node scripts/release.mjs tag {{ version }}

# Everything CI runs on this machine
ci: check test e2e
