# Podium — development tasks. Run `just` (or `just --list`) to see them.

# Show available recipes.
default:
    @just --list

# Run the app in development (Vite dev server + Tauri, hot reload).
dev:
    pnpm tauri dev

# Build + launch a debug .app signed with a STABLE identity (macOS).
# `just dev` ad-hoc-signs the binary with a fresh identity every rebuild, so the
# keychain re-prompts each run and "Always Allow" never sticks. This recipe
# keeps one signing identity across rebuilds, so a single "Always Allow"
# persists. Tradeoff: runs a bundle, so there's NO hot reload — use `just dev`
# for fast iteration and this when the keychain prompt is in your way.
# The identity lives in src-tauri/tauri.dev-signed.conf.json; edit it to use
# your own.
[doc("Build + launch a debug .app with a stable signing identity (macOS)")]
dev-signed:
    pnpm tauri build --debug --bundles app --config src-tauri/tauri.dev-signed.conf.json
    open target/debug/bundle/macos/Podium.app

# RELEASE build (.app + .dmg) — this is what CI ships. For a local install use
# `just install-signed` instead.
# Tauri's own dmg step uses HFS+, which is broken on macOS 26+, so we build
# the .app here and package the .dmg ourselves (APFS) via scripts/make-dmg.sh.
[doc("Release build: .app + .dmg (what CI ships)")]
build:
    pnpm install
    pnpm tauri build --bundles app
    ./scripts/make-dmg.sh

# Build a SIGNED release .app for personal install (macOS), then open Finder on
# it so you can drag it to /Applications. Reuses the stable identity in
# tauri.dev-signed.conf.json (that file only sets bundle.macOS.signingIdentity,
# so it's equally valid for release), which means keychain "Always Allow"
# persists and there's no Gatekeeper prompt on this machine.
# Only the .app is built (--bundles app) — no .dmg is needed for a personal
# install.
[doc("Build a signed release .app and open Finder on it (macOS)")]
build-signed:
    pnpm install
    pnpm tauri build --bundles app --config src-tauri/tauri.dev-signed.conf.json
    open target/release/bundle/macos

# Build the signed release and copy it straight into /Applications.
install-signed: build-signed
    rm -rf "/Applications/Podium.app"
    cp -R target/release/bundle/macos/Podium.app "/Applications/Podium.app"
    @echo "Installed → /Applications/Podium.app"

# Type-check the whole Rust workspace without building.
check:
    cargo check --workspace

# Run all tests (Rust workspace + frontend).
test:
    cargo test --workspace
    -pnpm test

# Run only the core unit tests.
test-core:
    cargo test -p podium-core

# Lint: clippy with warnings-as-errors + frontend lint.
lint:
    cargo clippy --workspace --all-targets -- -D warnings
    -pnpm lint

# Format Rust + frontend.
format:
    cargo fmt --all
    -pnpm format

# Set the version and sync it across Cargo.toml, tauri.conf.json, package.json.
# Usage: just version 0.2.0
[doc("Set the version and sync it across all manifests")]
version new:
    bash ./scripts/sync-version.sh --set {{new}}

# Verify the version is identical across all three manifests.
version-check:
    bash ./scripts/sync-version.sh --check
