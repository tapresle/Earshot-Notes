#!/usr/bin/env bash
# Builds the signed macOS app and disk image, and notarizes both when it can.
#
# Signing identity, in order of preference:
#   1. APPLE_SIGNING_IDENTITY, if set.
#   2. The first "Developer ID Application" certificate in your keychain.
#   3. The first "Apple Development" certificate in your keychain.
#   4. Ad-hoc signing ("-"), which runs on this Mac but makes macOS ask for
#      microphone and audio permissions again after each rebuild.
#
# Notarization needs a Developer ID identity and one set of credentials:
#   - An App Store Connect API key (recommended): APPLE_API_KEY (the key ID),
#     APPLE_API_ISSUER (the issuer ID), and APPLE_API_KEY_PATH (the .p8 file).
#   - An Apple ID: APPLE_ID, APPLE_PASSWORD (an app-specific password), and
#     APPLE_TEAM_ID.
# Tauri notarizes and staples the app; this script then signs, notarizes, and
# staples the disk image, and checks both with Gatekeeper. Without credentials,
# the build is signed but not notarized, and the script says so.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

first_identity() {
  security find-identity -v -p codesigning | grep -Eo "\"$1: [^\"]+\"" | head -n 1 | tr -d '"' || true
}

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  APPLE_SIGNING_IDENTITY="$(first_identity "Developer ID Application")"
fi
if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  APPLE_SIGNING_IDENTITY="$(first_identity "Apple Development")"
fi
export APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}"
echo "Signing with: $APPLE_SIGNING_IDENTITY"

# Arguments for notarytool, from the same variables Tauri reads.
NOTARY_AUTH=()
if [[ -n "${APPLE_API_KEY:-}${APPLE_API_ISSUER:-}${APPLE_API_KEY_PATH:-}" ]]; then
  : "${APPLE_API_KEY:?set APPLE_API_KEY, APPLE_API_ISSUER, and APPLE_API_KEY_PATH together}"
  : "${APPLE_API_ISSUER:?set APPLE_API_KEY, APPLE_API_ISSUER, and APPLE_API_KEY_PATH together}"
  : "${APPLE_API_KEY_PATH:?set APPLE_API_KEY, APPLE_API_ISSUER, and APPLE_API_KEY_PATH together}"
  if [[ ! -f "$APPLE_API_KEY_PATH" ]]; then
    echo "APPLE_API_KEY_PATH doesn't point to a file: $APPLE_API_KEY_PATH" >&2
    exit 1
  fi
  NOTARY_AUTH=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
elif [[ -n "${APPLE_ID:-}${APPLE_PASSWORD:-}${APPLE_TEAM_ID:-}" ]]; then
  : "${APPLE_ID:?set APPLE_ID, APPLE_PASSWORD, and APPLE_TEAM_ID together}"
  : "${APPLE_PASSWORD:?set APPLE_ID, APPLE_PASSWORD, and APPLE_TEAM_ID together}"
  : "${APPLE_TEAM_ID:?set APPLE_ID, APPLE_PASSWORD, and APPLE_TEAM_ID together}"
  NOTARY_AUTH=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
fi

NOTARIZE=false
if [[ ${#NOTARY_AUTH[@]} -gt 0 ]]; then
  if [[ "$APPLE_SIGNING_IDENTITY" != "Developer ID Application:"* ]]; then
    echo "Notarization credentials are set, but only a Developer ID Application identity can be notarized." >&2
    echo "Signing identity: $APPLE_SIGNING_IDENTITY" >&2
    exit 1
  fi
  NOTARIZE=true
fi

./scripts/fetch-native.sh
# The bundled third-party notices must match the current dependencies.
./scripts/generate-notices.sh --check
npm run tauri build

APP="$(ls -d src-tauri/target/release/bundle/macos/*.app)"
DMG="$(ls src-tauri/target/release/bundle/dmg/*.dmg)"

if [[ "$NOTARIZE" == true ]]; then
  echo
  echo "Notarizing the disk image"
  codesign --force --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$DMG"
  result="$(xcrun notarytool submit "$DMG" "${NOTARY_AUTH[@]}" --wait --output-format json)"
  status="$(plutil -extract status raw -o - - <<<"$result")"
  if [[ "$status" != "Accepted" ]]; then
    id="$(plutil -extract id raw -o - - <<<"$result" || true)"
    echo "Notarization of the disk image ended with status \"$status\"." >&2
    echo "For Apple's reasons, run: xcrun notarytool log $id <credentials>" >&2
    exit 1
  fi
  xcrun stapler staple "$DMG"

  echo
  echo "Checking with Gatekeeper"
  xcrun stapler validate "$APP"
  spctl --assess --type exec -vv "$APP"
  xcrun stapler validate "$DMG"
  spctl --assess --type open --context context:primary-signature -vv "$DMG"
  # The app inside the disk image is the copy people install.
  mount="$(mktemp -d)"
  hdiutil attach -nobrowse -readonly -quiet -mountpoint "$mount" "$DMG"
  trap 'hdiutil detach -quiet "$mount"' EXIT
  xcrun stapler validate "$mount/$(basename "$APP")"
  spctl --assess --type exec -vv "$mount/$(basename "$APP")"
fi

echo
echo "Built:"
echo "$APP"
echo "$DMG"
if [[ "$NOTARIZE" == true ]]; then
  echo "Notarized and stapled: the app and the disk image."
else
  echo "Not notarized: Gatekeeper blocks this build on other Macs until you choose Open Anyway."
  echo "To notarize, sign with a Developer ID Application certificate and set the credentials described at the top of $0."
fi
