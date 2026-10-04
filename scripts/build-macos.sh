#!/usr/bin/env bash
# Builds the signed macOS app and disk image.
#
# Signing identity, in order of preference:
#   1. APPLE_SIGNING_IDENTITY, if set.
#   2. The first "Developer ID Application" or "Apple Development" certificate in your keychain.
#   3. Ad-hoc signing ("-"), which runs on this Mac but makes macOS ask for
#      microphone and audio permissions again after each rebuild.
#
# Notarization isn't done here. It needs a paid Apple Developer account; see README.md.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  APPLE_SIGNING_IDENTITY="$(security find-identity -v -p codesigning \
    | grep -Eo '"(Developer ID Application|Apple Development): [^"]+"' \
    | head -n 1 | tr -d '"' || true)"
fi
export APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}"
echo "Signing with: $APPLE_SIGNING_IDENTITY"

./scripts/fetch-native.sh
npm run tauri build

echo
echo "Built:"
ls -d src-tauri/target/release/bundle/macos/*.app src-tauri/target/release/bundle/dmg/*.dmg 2>/dev/null
