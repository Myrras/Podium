#!/usr/bin/env bash
# Usage: next-version.sh <previous tag> <PR labels, comma-separated>
# Prints the next release tag: label `major` → vX+1.0.0, `minor` → vX.Y+1.0,
# anything else → patch.
set -euo pipefail

prev=${1:-}
labels=${2:-}

if [[ ! $prev =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "next-version: previous tag '$prev' is not vX.Y.Z" >&2
  exit 1
fi

IFS=. read -r major minor patch <<< "${prev#v}"

if [[ ",$labels," == *",major,"* ]]; then
  echo "v$((major + 1)).0.0"
elif [[ ",$labels," == *",minor,"* ]]; then
  echo "v$major.$((minor + 1)).0"
else
  echo "v$major.$minor.$((patch + 1))"
fi
