#!/usr/bin/env bash
# Manage the `## [Unreleased]` section of CHANGELOG.md.
#
#   changelog.sh unreleased              print its body; exit 1 if it has no `- ` entry
#   changelog.sh release X.Y.Z DATE      rename it to `## [X.Y.Z] - DATE` and add a
#                                        fresh empty `## [Unreleased]` above it
#
# CHANGELOG overrides the file (tests). perl/awk only, so it runs on macOS and ubuntu.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FILE="${CHANGELOG:-$ROOT/CHANGELOG.md}"
HEADING='## [Unreleased]'

has_heading() { grep -qxF "$HEADING" "$FILE"; }

case "${1:-}" in
  unreleased)
    has_heading || { echo "changelog: no '$HEADING' heading in $FILE" >&2; exit 1; }
    # Body = lines after the heading up to the next `## ` heading, outer blank lines trimmed.
    body=$(awk -v h="$HEADING" '
      $0 == h { on = 1; next }
      on && /^## / { exit }
      on { print }
    ' "$FILE" | awk 'NF { started = 1 } started' | perl -0pe 's/\s+\z/\n/')
    if ! grep -q '^- ' <<< "$body"; then
      echo "changelog: '$HEADING' in $FILE has no entries" >&2
      exit 1
    fi
    printf '%s\n' "$body"
    ;;
  release)
    ver=${2:-}
    date=${3:-}
    [[ $ver =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && $date =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] \
      || { echo "usage: changelog.sh release X.Y.Z YYYY-MM-DD" >&2; exit 2; }
    has_heading || { echo "changelog: no '$HEADING' heading in $FILE" >&2; exit 1; }
    V="$ver" D="$date" perl -i -pe '
      if (!$done && $_ eq "## [Unreleased]\n") { $_ = "## [Unreleased]\n\n## [$ENV{V}] - $ENV{D}\n"; $done = 1 }
    ' "$FILE"
    ;;
  *)
    echo "usage: changelog.sh unreleased | release X.Y.Z YYYY-MM-DD" >&2
    exit 2
    ;;
esac
