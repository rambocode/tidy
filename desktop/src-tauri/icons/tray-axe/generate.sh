#!/usr/bin/env bash
set -euo pipefail

# Generate the complete asset set outside the watched source tree. Tauri's
# development watcher otherwise starts compiling as soon as idle.png appears,
# while the axe frames referenced by include_bytes! may not exist yet.
tray_build_dir="$(mktemp -d "${TMPDIR:-/tmp}/tidy-tray-axe.XXXXXX")"
trap 'rm -rf -- "$tray_build_dir"' EXIT

# Slow lift, fast strike, short rebound. The pivot remains fixed at the end
# of the handle so every generated frame reads as one physical axe swing.
angles=(-10 -18 -28 -36 -40 -35 -25 -10 8 24 34 26 10 -4)

# The idle tray image is also authored ahead of runtime. This keeps the 512px
# application icon out of Tauri's macOS tray conversion path entirely.
magick ../icon.png -resize 44x44 "$tray_build_dir/idle.png"

for index in "${!angles[@]}"; do
  printf -v frame "axe-%02d.png" "$index"
  magick -background none background.svg \
    \( axe.svg \
      -virtual-pixel transparent \
      -define distort:viewport=44x44+0+0 \
      -distort SRT "22,36 1 ${angles[$index]} 22,36" \
    \) \
    -compose over -composite \
    "$tray_build_dir/$frame"
done

# This GIF is a review artifact. Runtime playback uses the individual PNG
# files because the macOS status-item API does not animate a GIF by itself.
magick -delay 9 -dispose background -loop 0 \
  "$tray_build_dir"/axe-*.png \
  "$tray_build_dir/axe-preview.gif"

# Publish only after every output has been generated successfully. Existing
# checked-in files remain readable throughout regeneration, so cargo never
# observes a missing include_bytes! dependency.
mv "$tray_build_dir/idle.png" idle.png
for index in "${!angles[@]}"; do
  printf -v frame "axe-%02d.png" "$index"
  mv "$tray_build_dir/$frame" "$frame"
done
mv "$tray_build_dir/axe-preview.gif" axe-preview.gif
