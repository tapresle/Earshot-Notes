#!/usr/bin/env bash
# Downloads the pinned needle engine and Whistle weights, and verifies their checksums.
# These binaries are not committed. Run this once after cloning, and again after a pin changes.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENDOR="$ROOT/src-tauri/vendor/needle/macos-arm64"
MODELS="$ROOT/src-tauri/resources/models"

NEEDLE_REPO="Cactus-Compute/needle3"
NEEDLE_REV="c7c415a3d1b3d929014bc6e866d51ebb971f7089" # engine 3.1.0
WHISTLE_REPO="Cactus-Compute/whistle"
WHISTLE_REV="b358ddadd89b7a713b5aa131f23032d3cca1b251" # weights 2.0.0

# repo revision remote-path local-path sha256
FILES=(
  "$NEEDLE_REPO $NEEDLE_REV macos-arm64/libneedle.a $VENDOR/libneedle.a a3b9163abe7b4bd52487c4005506cb35c5163bb9b9587af4ae07ed7587de697d"
  "$WHISTLE_REPO $WHISTLE_REV whistle.cact $MODELS/whistle.cact b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb"
)

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "Only macOS on Apple Silicon is supported so far." >&2
  exit 1
fi

for entry in "${FILES[@]}"; do
  read -r repo rev remote local expected <<<"$entry"
  if [[ -f "$local" ]] && [[ "$(shasum -a 256 "$local" | cut -d' ' -f1)" == "$expected" ]]; then
    echo "ok       $local"
    continue
  fi
  mkdir -p "$(dirname "$local")"
  echo "fetching $repo/$remote"
  curl -fsSL --retry 3 -o "$local.tmp" "https://huggingface.co/$repo/resolve/$rev/$remote"
  actual="$(shasum -a 256 "$local.tmp" | cut -d' ' -f1)"
  if [[ "$actual" != "$expected" ]]; then
    rm -f "$local.tmp"
    echo "Checksum mismatch for $remote: expected $expected, got $actual" >&2
    exit 1
  fi
  mv "$local.tmp" "$local"
  echo "ok       $local"
done
