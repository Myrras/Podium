# Podium — development tasks. Run `just` (or `just --list`) to see them.

# Show available recipes.
default:
    @just --list

# Run the app in development (Vite dev server + Tauri, hot reload).
dev:
    pnpm tauri dev

# Build the production desktop bundle (.app + .dmg).
# Tauri's own dmg step uses HFS+, which is broken on macOS 26+, so we build
# the .app here and package the .dmg ourselves (APFS) via scripts/make-dmg.sh.
build:
    pnpm install
    pnpm tauri build --bundles app
    ./scripts/make-dmg.sh

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
version new:
    bash ./scripts/sync-version.sh --set {{new}}

# Verify the version is identical across all three manifests.
version-check:
    bash ./scripts/sync-version.sh --check
