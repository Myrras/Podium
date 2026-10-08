#!/usr/bin/env bash
set -euo pipefail

script="$(dirname "$0")/next-version.sh"

check() {
  got=$("$script" "$1" "$2")
  if [ "$got" != "$3" ]; then
    echo "next-version '$1' '$2': expected $3, got $got"
    exit 1
  fi
}

check v2.1.2 "" v2.1.3
check v2.1.2 "bug,docs" v2.1.3
check v2.1.2 "minor" v2.2.0
check v2.1.2 "major" v3.0.0
check v2.1.2 "minor,major" v3.0.0
check v2.9.9 "minor" v2.10.0
check v2.1.9 "" v2.1.10
check v2.1.2 "not-minor" v2.1.3

if "$script" "" "" 2>/dev/null; then
  echo "next-version: expected failure without a previous tag"
  exit 1
fi
echo "next-version: ok"
