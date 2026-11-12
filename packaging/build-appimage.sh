#!/usr/bin/env bash
# Builds dist/Rollcopy-<version>-x86_64.AppImage from the release binary,
# using a pinned, checksummed linuxdeploy. Runs in CI and locally.
#
# The AppImage still needs these on the host (they can't be bundled):
#   usbmuxd (system daemon, talks to the phone) and ffmpeg (video conversion).
set -euo pipefail

ID=io.github.ankurk91.Rollcopy
LINUXDEPLOY_TAG=1-alpha-20251107-1
LINUXDEPLOY_SHA256=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)}"
TOOLS="$ROOT/target/appimage-tools"
APPDIR="$ROOT/target/AppDir"
OUT="$ROOT/dist"
BIN="$ROOT/target/release/rollcopy"

if [[ ! -x "$BIN" ]]; then
    cargo build --release -p rollcopy-app --manifest-path "$ROOT/Cargo.toml"
fi

mkdir -p "$TOOLS" "$OUT"
LD="$TOOLS/linuxdeploy-$LINUXDEPLOY_TAG.AppImage"
if [[ ! -x "$LD" ]]; then
    curl -fsSL -o "$LD.tmp" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/$LINUXDEPLOY_TAG/linuxdeploy-x86_64.AppImage"
    echo "$LINUXDEPLOY_SHA256  $LD.tmp" | sha256sum -c --quiet -
    chmod +x "$LD.tmp"
    mv "$LD.tmp" "$LD"
fi

rm -rf "${APPDIR:?}"
# No FUSE needed (CI runners don't have it).
export APPIMAGE_EXTRACT_AND_RUN=1
export LINUXDEPLOY_OUTPUT_VERSION="$VERSION"
cd "$OUT"
"$LD" --appdir "$APPDIR" \
    --executable "$BIN" \
    --desktop-file "$ROOT/packaging/$ID.desktop" \
    --icon-file "$ROOT/packaging/icons/$ID.svg" \
    --output appimage
ls -l "$OUT"/*.AppImage
