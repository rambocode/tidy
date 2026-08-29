#!/usr/bin/env bash
# Stamp the app icon onto the .dmg FILE itself.
#
# Why this is a separate step: tauri writes .VolumeIcon.icns inside the image,
# which only themes the MOUNTED volume. The .dmg sitting in ~/Downloads still
# draws the generic disk-image icon, because a file's custom icon lives in its
# resource fork and has to be attached after the image is built.
#
# Needs Rez/DeRez/SetFile from the Xcode command line tools. Missing tools are
# a warning, not an error: the DMG is still perfectly valid without the icon,
# so this must never fail a release build.
set -euo pipefail

DMG="${1:?usage: set-dmg-icon.sh <path-to-dmg> [path-to-icns]}"
ICNS="${2:-$(dirname "$0")/../src-tauri/icons/icon.icns}"

[ -f "$DMG" ]  || { echo "no such dmg: $DMG" >&2; exit 1; }
[ -f "$ICNS" ] || { echo "no such icns: $ICNS" >&2; exit 1; }

for tool in Rez DeRez SetFile sips; do
  command -v "$tool" >/dev/null || {
    echo "warning: $tool not found — leaving the generic DMG icon in place" >&2
    echo "         (install the Xcode command line tools: xcode-select --install)" >&2
    exit 0
  }
done

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# sips -i writes the icns's own artwork into its resource fork; DeRez then
# reads that fork back out as the 'icns' resource we append to the DMG.
cp "$ICNS" "$tmp/icon.icns"
sips -i "$tmp/icon.icns" >/dev/null
DeRez -only icns "$tmp/icon.icns" > "$tmp/icon.rsrc"
Rez -append "$tmp/icon.rsrc" -o "$DMG"
SetFile -a C "$DMG"   # 'C' = has a custom icon

echo "icon stamped onto $(basename "$DMG")"
