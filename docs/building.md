# Building Earshot Notes

To build Earshot Notes from source, you need:

- A Mac with Apple Silicon running macOS 26 or later
- Xcode, Rust, Node.js, and CMake

`AGENTS.md` lists every build, test, and lint command.

## Build and install

1. Fetch the native engine and models (checksums are verified):

   ```bash
   ./scripts/fetch-native.sh
   ```

2. Install JavaScript dependencies:

   ```bash
   npm install
   ```

3. Build the signed app and disk image:

   ```bash
   ./scripts/build-macos.sh
   ```

4. Open `src-tauri/target/release/bundle/dmg/Earshot Notes_<version>_aarch64.dmg` and drag the app to **Applications**. If an older version named `Earshot.app` is installed, delete it: both use the same meetings and models, so don't run them at the same time.

For development, run `npm run tauri dev` instead.

## Signing

`scripts/build-macos.sh` signs the app with the first of these it finds:

1. The identity in `APPLE_SIGNING_IDENTITY`, if you set it.
2. A "Developer ID Application" or "Apple Development" certificate in your keychain.
3. An ad-hoc signature.

A free **Apple Development** certificate is enough for your own Mac. To create one, open Xcode, choose **Xcode** > **Settings** > **Accounts**, sign in with your Apple ID, select your **Personal Team**, click **Manage Certificates**, and add an **Apple Development** certificate. If `security find-identity -v -p codesigning` reports no valid identities afterward, install Apple's [Worldwide Developer Relations G3 intermediate certificate](https://www.apple.com/certificateauthority/).

With an ad-hoc signature the app also runs, but macOS treats each build as a new app, so it asks for microphone and audio recording permission again after every rebuild.

## Notarizing

Releases are notarized, so the disk image from a GitHub release opens on any Mac with Apple Silicon and macOS 26.

A build you make yourself is notarized only when it's signed with a **Developer ID Application** certificate and you set notarization credentials before running `scripts/build-macos.sh`. Use an App Store Connect API key (`APPLE_API_KEY`, `APPLE_API_ISSUER`, and `APPLE_API_KEY_PATH`), or an Apple ID (`APPLE_ID`, `APPLE_PASSWORD` as an app-specific password, and `APPLE_TEAM_ID`). The script then notarizes and staples both the app and the disk image, and checks them with Gatekeeper.

Otherwise the build isn't notarized, and Gatekeeper blocks it on other Macs. Open it once, then choose **System Settings** > **Privacy & Security** > **Open Anyway**, or remove the quarantine flag:

```bash
xattr -dr com.apple.quarantine "/Applications/Earshot Notes.app"
```

## Permissions after a rebuild

If recording goes silent after you install a new build, open **System Settings** > **Privacy & Security**, and turn **Earshot Notes** off and on again under **Microphone** and **Screen & System Audio Recording**.
