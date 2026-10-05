# Earshot

A meeting notes app that runs entirely on your Mac. It transcribes your microphone and the other side of your calls with [Cactus Whistle](https://huggingface.co/Cactus-Compute/whistle), gives you a notepad during the meeting, and on request turns your notes and the transcript into meeting notes with a local language model through [llama.cpp](https://github.com/ggml-org/llama.cpp). Nothing leaves your computer. The only network request is the optional, one-time model download you start yourself.

## Requirements

- A Mac with Apple Silicon running macOS 26 or later
- Xcode, Rust, Node.js, and CMake to build

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

4. Open `src-tauri/target/release/bundle/dmg/Earshot_0.1.0_aarch64.dmg` and drag the app to **Applications**.

For development, run `npm run tauri dev` instead. `AGENTS.md` lists every build and test command.

Release notes for each version are in [`docs/releases`](docs/releases).

## Signing

`scripts/build-macos.sh` signs the app with the first of these it finds:

1. The identity in `APPLE_SIGNING_IDENTITY`, if you set it.
2. A "Developer ID Application" or "Apple Development" certificate in your keychain.
3. An ad-hoc signature.

A free **Apple Development** certificate is enough for your own Mac. To create one, open Xcode, choose **Xcode** > **Settings** > **Accounts**, sign in with your Apple ID, select your **Personal Team**, click **Manage Certificates**, and add an **Apple Development** certificate. If `security find-identity -v -p codesigning` reports no valid identities afterward, install Apple's [Worldwide Developer Relations G3 intermediate certificate](https://www.apple.com/certificateauthority/).

With an ad-hoc signature the app also runs, but macOS treats each build as a new app, so it asks for microphone and audio recording permission again after every rebuild.

### Running the app on another Mac

The app isn't notarized, so Gatekeeper blocks it on other Macs. Open it once, then choose **System Settings** > **Privacy & Security** > **Open Anyway**, or remove the quarantine flag:

```bash
xattr -dr com.apple.quarantine "/Applications/Earshot.app"
```

Notarization needs a paid Apple Developer account and a **Developer ID Application** certificate. With those, set `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password), and `APPLE_TEAM_ID` before running `scripts/build-macos.sh`, and Tauri notarizes the build.

## Permissions

On first launch, a checklist walks you through these:

- **Microphone:** needed for your side of the conversation.
- **Screen & System Audio Recording:** needed for the other side of your calls. The checklist asks for it up front. If you decline, recording continues with only your microphone and says how to turn it on.

If recording goes silent after you install a new build, open **System Settings** > **Privacy & Security**, and turn **Earshot** off and on again under **Microphone** and **Screen & System Audio Recording**.

## Summaries

Summaries need a local model. In **Settings**, or on a meeting's **Summary** tab, download the recommended model (Gemma 4 E2B, 3.1 GB) or one of the alternatives (Qwen3 4B Instruct, Phi-4 mini, Gemma 4 E4B, Qwen3 8B, Ministral 3 8B, or Ministral 3 14B), or import any GGUF file you already have. Downloads resume if interrupted and are verified against a pinned SHA-256 checksum before use. To free disk space, click **Remove** next to an installed model in **Settings**.

## License

Earshot is free software, licensed under the [GNU General Public License, version 3](LICENSE) only (`GPL-3.0-only`). You can use, study, change, and share it. If you distribute a modified version, you must make its source available under the same license.

The app bundles the needle engine and Whistle model (Apache 2.0) and links llama.cpp (MIT), both compatible with GPL 3.0. The `needle-sys` crate, which includes needle's C header, stays under Apache 2.0. All of these license texts, including Earshot's, are in the app bundle under `Contents/Resources/licenses`.
