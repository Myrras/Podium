# Podium — development tasks. Run `just` (or `just --list`) to see them.
# Shared recipes (rust, frontend, ci, build-signed, sweep) come from the
# ci-workflows clone: `ln -s /path/to/ci-workflows ~/.ci-workflows`.
import? '~/.ci-workflows/common.just'

# Share one target/ across the main checkout and every git worktree. A fresh
# worktree otherwise starts cold and rebuilds every dependency (minutes);
# pointing it at the main checkout's target/ keeps them warm.
# ponytail: cargo locks the target dir, so two builds in different worktrees
# serialise instead of running in parallel — still far cheaper than a cold
# rebuild. Falls back to ./target outside a git checkout.
export CARGO_TARGET_DIR := `dirname "$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || echo "$PWD/.git")"` / "target"

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
    open "$CARGO_TARGET_DIR/debug/bundle/macos/Podium.app"

# RELEASE build (.app + .dmg) — this is what CI ships. For a local install use
# `just install-signed` instead.
# Tauri's own dmg step uses HFS+, which is broken on macOS 26+, so we build
# the .app here and package the .dmg ourselves (APFS) via the shared make-dmg.sh.
[doc("Release build: .app + .dmg (what CI ships)")]
build:
    pnpm install
    pnpm tauri build --bundles app
    ~/.ci-workflows/scripts/make-dmg.sh

# Build the signed release (shared `build-signed`) and copy it straight into /Applications.
install-signed:
    just build-signed
    rm -rf "/Applications/Podium.app"
    cp -R "$CARGO_TARGET_DIR/release/bundle/macos/Podium.app" "/Applications/Podium.app"
    @echo "Installed → /Applications/Podium.app"

# Type-check the whole Rust workspace without building.
check:
    cargo check --workspace

# Run only the core unit tests.
test-core:
    cargo test -p podium-core

# Format Rust + frontend.
format:
    cargo fmt --all
    pnpm format

# Run by the nightly integration workflow. Standalone: must not use common.just,
# because CI has no common.just import.

# Opt-in Windows + WSL tests (#[ignore]d; need a real WSL distro).
test-integration:
    cargo test -p podium-core --test wsl_backend --test wsl_native_drives -- --ignored --test-threads=1

# Set the version and sync it across Cargo.toml, tauri.conf.json, package.json.
# Usage: just version 0.2.0
[doc("Set the version and sync it across all manifests")]
version new:
    bash ~/.ci-workflows/scripts/sync-version.sh --set {{ new }}

# Verify the version is identical across all three manifests.
version-check:
    bash ~/.ci-workflows/scripts/sync-version.sh --check
