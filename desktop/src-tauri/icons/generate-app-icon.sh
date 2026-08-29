#!/usr/bin/env bash
# Regenerate the application icon set (icon.icns + the PNG sizes tauri.conf.json
# lists under bundle.icon) from the single 1024x1024 source icon.png.
#
# Why this exists: macOS reads an app bundle's icon from a .icns file only.
# Shipping just a PNG makes Finder, the Dock, and the DMG fall back to the
# generic application icon, which is exactly the "no logo after install" bug.
# Run it after changing icon.png, then commit the generated files.
set -euo pipefail

cd "$(dirname "$0")"
SRC="icon.png"
[ -f "$SRC" ] || { echo "missing $SRC" >&2; exit 1; }

# iconutil only accepts a directory named *.iconset holding Apple's exact
# icon_<W>x<H>[@2x].png names; any other name makes it refuse the folder.
ICONSET="$(mktemp -d)/icon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$SRC" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$SRC" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil --convert icns "$ICONSET" --output icon.icns
rm -rf "$(dirname "$ICONSET")"

# Flat PNGs for the cross-platform entries of bundle.icon.
sips -z 32 32 "$SRC" --out 32x32.png >/dev/null
sips -z 128 128 "$SRC" --out 128x128.png >/dev/null
sips -z 256 256 "$SRC" --out "128x128@2x.png" >/dev/null

echo "generated: icon.icns 32x32.png 128x128.png 128x128@2x.png"
