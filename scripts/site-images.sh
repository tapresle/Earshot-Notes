#!/bin/bash
# Builds the website's images (site/images/) from the app icon and the UI
# screenshots. Run it after the icon changes or after `npm run screenshots`.
# Uses sips, which ships with macOS.
set -euo pipefail

cd "$(dirname "$0")/.."
icon=src-tauri/icons/source/icon.png
out=site/images
mkdir -p "$out"

# Icons: the page's own icon, the favicon, and the home screen icon.
sips -z 256 256 "$icon" --out "$out/icon-256.png" >/dev/null
sips -z 32 32 "$icon" --out "$out/favicon-32.png" >/dev/null
sips -z 180 180 "$icon" --out "$out/apple-touch-icon.png" >/dev/null

# Link preview (Open Graph): the icon centered on the icon's violet, 1200 x 630.
sips -z 440 440 "$icon" --out "$out/og.png" >/dev/null
# sips prints a debugging line for the pad color.
sips -p 630 1200 --padColor 6F00FF "$out/og.png" --out "$out/og.png" >/dev/null 2>&1

# Screenshots, in light and dark.
for name in meeting summary; do
  for mode in light dark; do
    cp "docs/screenshots/$name-$mode.png" "$out/$name-$mode.png"
  done
done

echo "Wrote $out"
