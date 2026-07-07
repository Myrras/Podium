#!/usr/bin/env bash
# Build a distributable .dmg from the Tauri-built .app.
#
# Why this exists: Tauri's bundled `bundle_dmg.sh` calls
# `hdiutil create -fs HFS+ …`, which fails with
# "hdiutil: create failed - no mountable file systems" on macOS 26+
# (HFS+ image creation via -srcfolder is broken there). Tauri exposes no
# config knob for the DMG filesystem, so we skip its dmg target
# (`tauri build --bundles app`) and build the image ourselves with APFS,
# which works. Output name matches Tauri's convention so nothing downstream
# needs to change.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$REPO_ROOT/target/release/bundle/macos/Podium.app"
OUT_DIR="$REPO_ROOT/target/release/bundle/dmg"

if [[ ! -d "$APP" ]]; then
	echo >&2 "error: $APP not found — run 'pnpm tauri build --bundles app' first."
	exit 1
fi

VERSION="$(grep -m1 '^version' "$REPO_ROOT/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/')"
case "$(uname -m)" in
	arm64) ARCH="aarch64" ;;
	x86_64) ARCH="x64" ;;
	*) ARCH="$(uname -m)" ;;
esac

DMG="$OUT_DIR/Podium_${VERSION}_${ARCH}.dmg"
VOLNAME="Podium"

# Assemble a staging tree: the app plus a drop-link to /Applications.
STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT
cp -R "$APP" "$STAGING/"
ln -s /Applications "$STAGING/Applications"

mkdir -p "$OUT_DIR"
rm -f "$DMG"

echo "Creating APFS disk image → $DMG"
hdiutil create \
	-volname "$VOLNAME" \
	-srcfolder "$STAGING" \
	-fs APFS \
	-format UDZO \
	-ov \
	"$DMG"

echo "Built $DMG"
