# Podium — development tasks. Run `just` (or `just --list`) to see them.
# Shared recipes (dev, build, check, test-core, format, version, ci, ...; see
# `just --list`) come from the ci-workflows clone: `ln -s /path/to/ci-workflows ~/.ci-workflows`.
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

# Run by the nightly integration workflow. Standalone: must not use common.just,
# because CI has no common.just import.

# Opt-in Windows + WSL tests (#[ignore]d; need a real WSL distro).
test-integration:
    cargo test -p podium-core --test wsl_backend --test wsl_native_drives -- --ignored --test-threads=1
