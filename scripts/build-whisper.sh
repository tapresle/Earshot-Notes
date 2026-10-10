#!/usr/bin/env bash
# Builds whisper.cpp from source at a pinned commit, with the C interface in
# src-tauri/crates/whisper-sys/native, into one dynamic library:
# src-tauri/vendor/whisper/macos-arm64/libearshot_whisper.dylib.
#
# The library carries its own copy of ggml and exports only the earshot_whisper_*
# functions. llama.cpp's copy of ggml is linked into the app itself, and two
# copies in one image would clash; in a separate library with hidden symbols,
# they can't.
#
# fetch-native.sh runs this. It skips the build when the library is already
# built from the same commit and interface.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENDOR="$ROOT/src-tauri/vendor/whisper"
SRC="$VENDOR/src"
BUILD="$VENDOR/build"
OUT="$VENDOR/macos-arm64/libearshot_whisper.dylib"
SHIM="$ROOT/src-tauri/crates/whisper-sys/native"

WHISPER_REPO="https://github.com/ggml-org/whisper.cpp.git"
WHISPER_TAG="v1.9.5"
WHISPER_COMMIT="d1be6fde11ac6e0407606b4e42fe72d34add8037"
# Keep equal to MACOSX_DEPLOYMENT_TARGET in src-tauri/.cargo/config.toml.
DEPLOYMENT_TARGET="26.0"

# Rebuild when the pin, the interface, or this script changes.
stamp="$(cat "$0" "$SHIM/earshot_whisper.h" "$SHIM/earshot_whisper.c" | shasum -a 256 | cut -d' ' -f1)"
if [[ -f "$OUT" && -f "$OUT.stamp" && "$(cat "$OUT.stamp")" == "$stamp" ]]; then
  echo "ok       $OUT"
  exit 0
fi

if [[ "$(git -C "$SRC" rev-parse HEAD 2>/dev/null || true)" != "$WHISPER_COMMIT" ]]; then
  echo "fetching whisper.cpp $WHISPER_TAG"
  rm -rf "$SRC"
  git clone --quiet --depth 1 --branch "$WHISPER_TAG" "$WHISPER_REPO" "$SRC"
  actual="$(git -C "$SRC" rev-parse HEAD)"
  if [[ "$actual" != "$WHISPER_COMMIT" ]]; then
    rm -rf "$SRC"
    echo "whisper.cpp $WHISPER_TAG is commit $actual, expected $WHISPER_COMMIT" >&2
    exit 1
  fi
fi

echo "building whisper.cpp $WHISPER_TAG"
# GGML_NATIVE=OFF targets every Apple Silicon Mac, not only this one.
cmake -S "$SRC" -B "$BUILD" -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_OSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" -DCMAKE_OSX_ARCHITECTURES=arm64 \
  -DBUILD_SHARED_LIBS=OFF -DGGML_NATIVE=OFF -DGGML_OPENMP=OFF \
  -DGGML_METAL=ON -DGGML_METAL_EMBED_LIBRARY=ON -DGGML_BLAS=ON \
  -DWHISPER_BUILD_EXAMPLES=OFF -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF \
  >/dev/null
cmake --build "$BUILD" --config Release -j "$(sysctl -n hw.ncpu)" >/dev/null

libs=()
while IFS= read -r lib; do libs+=("$lib"); done < <(find "$BUILD" -name '*.a' | sort)

mkdir -p "$(dirname "$OUT")"
clang -c -std=c11 -O2 -Wall -Wextra -Werror -arch arm64 -mmacosx-version-min="$DEPLOYMENT_TARGET" \
  -I "$SRC/include" -I "$SRC/ggml/include" \
  -o "$BUILD/earshot_whisper.o" "$SHIM/earshot_whisper.c"
printf '_earshot_whisper_load\n_earshot_whisper_transcribe\n_earshot_whisper_free\n_earshot_whisper_last_error\n' \
  >"$BUILD/exports.txt"
clang++ -dynamiclib -arch arm64 -mmacosx-version-min="$DEPLOYMENT_TARGET" \
  -install_name @rpath/libearshot_whisper.dylib \
  -exported_symbols_list "$BUILD/exports.txt" -dead_strip \
  -o "$OUT.tmp" "$BUILD/earshot_whisper.o" -Wl,-all_load "${libs[@]}" \
  -framework Accelerate -framework Foundation -framework Metal -framework MetalKit
strip -x "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
echo "$stamp" >"$OUT.stamp"
echo "ok       $OUT"
