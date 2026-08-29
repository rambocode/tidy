#!/usr/bin/env bash
# Regenerate the DMG install-window background from background.svg.
#
# Output is a multi-resolution TIFF, not a PNG: Finder scales a 1x background
# up on Retina displays, which visibly softens the text. tiffutil packs the 1x
# and 2x renders into one file that Finder picks the right page from, so the
# copy stays sharp on both. Run after editing background.svg, then commit.
#
# Requires librsvg (brew install librsvg); tiffutil ships with macOS.
set -euo pipefail

cd "$(dirname "$0")"
command -v rsvg-convert >/dev/null || { echo "need rsvg-convert (brew install librsvg)" >&2; exit 1; }

# Must stay in sync with bundle.macOS.dmg.windowSize in tauri.conf.json:
# the art is sized to the window's CONTENT area, not the window frame.
W=660
H=400

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
rsvg-convert -w "$W"          -h "$H"          -o "$tmp/1x.png" background.svg
rsvg-convert -w "$((W * 2))"  -h "$((H * 2))"  -o "$tmp/2x.png" background.svg
tiffutil -cathidpicheck "$tmp/1x.png" "$tmp/2x.png" -out background.tiff >/dev/null

# Keep a plain PNG next to it purely for previewing the art in a browser or
# image viewer; the bundle only ever reads the TIFF.
cp "$tmp/1x.png" background.png

echo "generated: background.tiff (${W}x${H} @1x+@2x), background.png (preview)"
