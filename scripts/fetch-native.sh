#!/usr/bin/env bash
# Downloads the pinned needle engine and Whistle weights, and verifies their checksums,
# then builds whisper.cpp (scripts/build-whisper.sh).
# These binaries are not committed. Run this once after cloning, and again after a pin changes.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENDOR="$ROOT/src-tauri/vendor/needle/macos-arm64"
MODELS="$ROOT/src-tauri/resources/models"

NEEDLE_REPO="Cactus-Compute/needle3"
NEEDLE_REV="2ae11323dc000f5e70c49f7403efa6af12ba9e67" # engine 3.1, production build of 2026-10-05 (adds live streaming)
WHISTLE_REPO="Cactus-Compute/whistle"
WHISTLE_REV="b358ddadd89b7a713b5aa131f23032d3cca1b251" # weights 2.0.0
# A tiny chat model for automated tests of the summary engine. Not shipped.
TEST_LLM_REPO="bartowski/SmolLM2-135M-Instruct-GGUF"
TEST_LLM_REV="09816acd5d99df7be770d85ea30822623dab342c"
# A tiny Whisper model for automated tests of the Whisper engine. Not shipped.
TEST_WHISPER_REPO="ggerganov/whisper.cpp"
TEST_WHISPER_REV="5359861c739e955e79d9a303bcbc70fb988958b1"
TEST_MODELS="$ROOT/src-tauri/tests/models"

# repo revision remote-path local-path sha256
FILES=(
  "$NEEDLE_REPO $NEEDLE_REV macos-arm64/libneedle.a $VENDOR/libneedle.a 98da47c15e1065b4cdc7ddc55e825be78414d4586832db3becaeded39a373df4"
  "$WHISTLE_REPO $WHISTLE_REV whistle.cact $MODELS/whistle.cact b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb"
  "$TEST_LLM_REPO $TEST_LLM_REV SmolLM2-135M-Instruct-Q4_K_M.gguf $TEST_MODELS/smollm2-135m-instruct-q4_k_m.gguf 2e8040ceae7815abe0dcb3540b9995eaa1fa0d2ca9e797d0a635ae4433c68c2d"
  "$TEST_WHISPER_REPO $TEST_WHISPER_REV ggml-tiny.en-q5_1.bin $TEST_MODELS/ggml-tiny.en-q5_1.bin c77c5766f1cef09b6b7d47f21b546cbddd4157886b3b5d6d4f709e91e66c7c2b"
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

"$ROOT/scripts/build-whisper.sh"
