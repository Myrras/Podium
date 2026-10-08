#!/usr/bin/env bash
set -euo pipefail

script="$(dirname "$0")/changelog.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export CHANGELOG="$tmp/CHANGELOG.md"

fail() { echo "changelog: $*"; exit 1; }

cat > "$CHANGELOG" <<'MD'
# Changelog

Intro.

## [Unreleased]

### Added

- **A fictitious widget.** It does widget things.

## [1.0.0] - 2026-01-01

### Fixed

- **An old fix.**
MD

got=$("$script" unreleased)
want=$'### Added\n\n- **A fictitious widget.** It does widget things.'
[ "$got" = "$want" ] || fail "unreleased body: got [$got]"

"$script" release 1.1.0 2026-02-03
want_file=$'# Changelog\n\nIntro.\n\n## [Unreleased]\n\n## [1.1.0] - 2026-02-03\n\n### Added\n\n- **A fictitious widget.** It does widget things.\n\n## [1.0.0] - 2026-01-01\n\n### Fixed\n\n- **An old fix.**'
[ "$(cat "$CHANGELOG")" = "$want_file" ] || fail "release rewrote the file wrongly: $(cat "$CHANGELOG")"

# Freshly released: empty section → unreleased fails (the CI idempotency guard).
"$script" unreleased >/dev/null 2>&1 && fail "empty section should fail"

# Subheadings without bullets still count as empty.
printf '# Changelog\n\n## [Unreleased]\n\n### Added\n\n## [1.0.0] - 2026-01-01\n\n- old\n' > "$CHANGELOG"
"$script" unreleased >/dev/null 2>&1 && fail "subheading-only section should fail"

# Missing heading fails for both subcommands.
printf '# Changelog\n\n## [1.0.0] - 2026-01-01\n\n- old\n' > "$CHANGELOG"
"$script" unreleased >/dev/null 2>&1 && fail "missing heading should fail (unreleased)"
"$script" release 1.0.1 2026-01-02 >/dev/null 2>&1 && fail "missing heading should fail (release)"

echo "changelog: ok"
