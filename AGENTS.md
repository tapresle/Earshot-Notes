# Earshot Notes

Earshot Notes is a local-first meeting notes app. It transcribes your microphone and system audio on the device with Cactus Whistle, gives you a notepad during the meeting, and on request summarizes your notes and the transcript with a local LLM. It makes no network calls at runtime.

**Naming:** the app is "Earshot Notes", always in full in anything the app shows, including permission prompts and error messages. Docs may shorten it to "Earshot" after the first mention. The bundle identifier stays `com.tedpresley.earshot`, because the app data folder, the log folder, and macOS privacy permissions are keyed by it; changing it would hide existing meetings and models and ask for permissions again. Crate, binary, and package names stay `earshot`. The repository is `tapresle/Earshot-Notes`; its name affects only links, not the app.

macOS (Apple Silicon) is the first target. Windows and Linux follow.

## Stack

- **Shell:** Tauri 2. The Rust core lives in `src-tauri/`.
- **UI:** React 19, TypeScript, and Vite in `src/`.
- **Transcription:** Cactus Whistle through the prebuilt needle engine. `src-tauri/crates/needle-sys` holds the raw FFI, and `src-tauri/src/transcribe/whistle.rs` is the safe wrapper.
- **Summaries:** llama.cpp embedded through `llama-cpp-2` with Metal, in `src-tauri/src/summarize/`. Prompts are rendered with each model's own chat template through `minijinja`. Model files are managed in `src-tauri/src/models/`.
- **Storage:** SQLite through `rusqlite`, in `src-tauri/src/store/`. The database lives in the app data directory as `earshot.db`.

The decision records in `docs/decisions/` explain why each piece was chosen.

## Commands

Run all commands from the repository root unless noted.

| Task | Command |
| --- | --- |
| Fetch the native engine and models (after cloning, or when a pin changes) | `./scripts/fetch-native.sh` |
| Install JS dependencies | `npm install` |
| Run the app in development | `npm run tauri dev` |
| Build the signed app and disk image | `./scripts/build-macos.sh` |
| Regenerate the third-party license notices (needs `cargo install cargo-about --locked --features cli` and `npm install`) | `./scripts/generate-notices.sh` |
| Frontend lint | `npm run lint` |
| Frontend typecheck | `npm run typecheck` |
| Frontend tests | `npm test` |
| Rust lint | `cargo clippy --workspace --all-targets -- -D warnings` (in `src-tauri/`) |
| Rust format check | `cargo fmt --check` (in `src-tauri/`) |
| Rust tests | `cargo test --workspace` (in `src-tauri/`) |
| Update the UI screenshots (needs `npx playwright install chromium` once) | `npm run screenshots` |
| Check the UI screenshots are current | `npm run screenshots:check` |
| Check for newer versions of the pinned engine, models, and llama.cpp (reads Hugging Face and crates.io) | `node scripts/upstream/check.ts` |

Before you hand off work, run the lint, typecheck, test, format, and screenshot check commands. All of them must pass. When you change the UI on purpose, run `npm run screenshots` and commit the updated images with the change.

## Continuous integration

GitHub Actions runs these checks on every pull request to `main` and every push to `main`:

| Workflow | Runner | Jobs |
| --- | --- | --- |
| `.github/workflows/ci.yml` | `ubuntu-latest` | Frontend lint, typecheck, and tests; `./scripts/generate-notices.sh --check` |
| `.github/workflows/screenshots.yml` | `macos-26` | `npm run screenshots:check`; on failure, uploads the rendered screenshots and their differences as the `screenshots` artifact |
| `.github/workflows/rust.yml` | `macos-26` (Apple Silicon) | `./scripts/fetch-native.sh`, then `cargo fmt --check`, clippy, and `cargo test --workspace` (summary tests on the CPU, see **Summary tests**) |

The screenshots workflow runs on macOS because the UI uses the system font, and only when the UI, the preview, or the screenshots change. The Rust workflow needs macOS 26 on Apple Silicon, because the prebuilt needle engine supports nothing else. macOS minutes cost more, so it runs only when `src-tauri/`, `scripts/fetch-native.sh`, or the workflow itself changes. It caches the fetched native files (keyed on `fetch-native.sh`, whose checksums are still verified) and the cargo build (keyed on the Rust version and `Cargo.lock`). A newer run on the same pull request cancels the older one. To rerun a failed job, use **Re-run failed jobs** on the run's page, or `gh run rerun <run id> --failed`. Workflows keep `permissions: contents: read` and pin each action to a full commit SHA, with its version in a comment.

Two more jobs keep the pins current (see **Updating pinned versions**). `.github/workflows/upstream.yml` runs `scripts/upstream/check.ts` every Monday, or from **Run workflow** on its page, and keeps one open issue, "Upstream updates available", current; it also has `issues: write`, for that issue only. Dependabot (`.github/dependabot.yml`) checks weekly and opens one grouped pull request each for Cargo, npm, and GitHub Actions minor and patch updates, and a separate pull request for each major update.

## Conventions

- **UI features** live under `src/features/<feature>/`. Each feature keeps its components, hooks, and tests together.
- **Rust modules** under `src-tauri/src/`: `audio` (microphone and system capture, downmix, resampling), `chunker` (speech segmentation), `transcribe` (Whistle and its worker thread), `recording` (start, stop, and the capture-to-worker pipeline), `summarize` (prompting, the llama.cpp generator, and the background summarizer), `models` (known models, download, and GGUF import), `export` (Markdown rendering and file writing), `tray` (the menu bar icon and its menu), `window` (showing and hiding the main window and the Dock icon), `updates` (checking GitHub for a newer release), `store`, and `commands`. Tauri command handlers in `commands` stay thin and call into the other modules. Synchronous commands run on the main thread, so a command that touches the database, the disk, or another thread is `async` and does that work through `commands::blocking`, which moves it to the blocking thread pool. `Store` serializes access to its one connection with a mutex, and shared state such as `Store`, `Recorder`, and `Summarizer` is managed as an `Arc` so the work can take its own handle.
- **UI and core communication:** the UI calls Tauri commands for requests, and the core pushes live updates as Tauri events. Current events: `transcript:segment`, `recording:changed`, `recording:issue`, `summary:progress`, `summary:text`, `summary:done`, `summary:failed`, `model:download-progress`, `model:download-done`, and `model:download-failed`.
- **Menu bar and window:** Earshot Notes runs in the menu bar (`tray.rs`). Closing the window hides it and the Dock icon instead of quitting (`window.rs`), so recordings continue. Show and hide the Dock icon only through `macos_audio::show_dock_icon` and `hide_dock_icon`, never Tauri's `set_activation_policy`: the shim hides the app first and drops the Dock icon once it's inactive, and waits a second after a show, because otherwise macOS redraws the whole menu bar and every menu bar icon jumps; **Quit** in the menu bar or the app menu exits, and `RunEvent::Exit` stops any recording first. The window starts hidden (`visible: false` in `tauri.conf.json`) and setup shows it, unless macOS opened the app at login. Start and stop every recording through `commands::recording::start` and `stop`, never `recording::start_recording` directly: they emit `recording:changed` and update the menu bar, which is how the window learns about recordings started or stopped from the menu bar. Opening at login is macOS's own setting (`SMAppService`), read each time rather than stored in `settings`, and works only for the bundled app.
- **Audio pipeline:** capture threads must never block. They drop buffers when the pipeline falls behind. Captures send `audio::Buffer`, which carries the rate its samples were captured at, because a device's rate can change mid-recording (a Bluetooth headset drops to 16 or 24 kHz when its microphone opens). System audio follows the default output device: `audio/system.rs` watches it through `macos_audio::OutputDeviceWatcher` and rebuilds the tap on the new device, sending silence for the time between the two taps' buffers so transcript times stay in step; a failed rebuild reports a `recording:issue`. Never assume the rate from `CaptureFormat` stays fixed: `recording::run_pipeline` flushes the old resampler with `To16k::finish` and creates a new one when the rate changes, which keeps transcript times exact. Whistle receives 16 kHz mono segments of at most 28 s from `chunker`. System audio uses a lower absolute speech floor than the microphone (`SYSTEM_MIN_SPEECH_RMS` and `MIC_MIN_SPEECH_RMS` in `chunker.rs`), because call audio is quieter and has no microphone hiss. Those are the **Normal** floors: each source has a stored `chunker::Sensitivity` (Low, Normal, or High) that `start_recording` reads and passes to `run_pipeline` through `Pipeline::new`. Transcript times are offsets from `meetings.started_at`. When the **Keep audio recordings** setting is on, `src-tauri/src/recordings.rs` saves each source's 16 kHz stream, before `chunker`, to `<app data>/recordings/<meeting id>/<offset>-<source>.wav` from a writer thread; the pipeline only hands samples over without waiting, and audio that doesn't fit is written as silence so the file keeps its timing. These files are full conversations: never log, export, or upload them. Whistle can invent short phrases such as "Thank you." from clips with little speech, and reports no confidence, so `chunker` measures each segment's speech time and loudness margin (`chunker::Speech`) and `transcribe/phantom.rs` drops a listed phrase only when its clip was weak.
- **Packaging:** `scripts/build-macos.sh` signs with `APPLE_SIGNING_IDENTITY`, else the first Developer ID Application certificate in the keychain, else the first Apple Development certificate, else ad-hoc. With a Developer ID identity and notarization credentials (an App Store Connect API key or an Apple ID, described at the top of the script), Tauri notarizes and staples the app, and the script signs, notarizes, and staples the disk image, then fails unless Gatekeeper accepts the app, the disk image, and the app inside it. Credentials come from the environment, never from the repository. Without them, the build is signed but not notarized, and the script says so. The build uses the hardened runtime, so `src-tauri/Entitlements.plist` must keep `com.apple.security.device.audio-input`, or microphone input is silently denied. The deployment target is macOS 26 for every build, set in `src-tauri/.cargo/config.toml`; keep it equal to `minimumSystemVersion` in `tauri.conf.json`. After changing it, run `cargo clean` in `src-tauri/`: llama.cpp's build script doesn't notice the change, and a stale build fails to link with an undefined `__isPlatformVersionAtLeast`. Bundled license texts live in `src-tauri/resources/licenses/`. `third-party-notices.txt` there is generated by `scripts/generate-notices.sh` from the normal Rust dependencies for `aarch64-apple-darwin` (cargo-about, configured in `scripts/notices/about.toml`) and the production npm dependencies in `package-lock.json`; never edit it by hand. A dependency under a license missing from `accepted` in `about.toml` fails the script, so review the license before adding it there. An npm package published without a license file needs one in `scripts/notices/npm-fallback/<package>/`. `build-macos.sh` runs the script with `--check` and stops if the file is out of date.
- **Native macOS code:** `src-tauri/crates/macos-audio` holds the Objective-C system audio tap, the microphone permission calls, and the login item (`SMAppService`) calls, compiled by its `build.rs` with `cc` under `-Wall -Wextra -Werror`. Keep its C interface in `native/audio_tap.h` and its Rust bindings in `src/lib.rs` in sync.
- **Permissions:** the bundled app declares `NSMicrophoneUsageDescription` and `NSAudioCaptureUsageDescription` in `src-tauri/Info.plist`, and requires macOS 26, the oldest version the prebuilt needle engine supports. System audio permission is read and requested through the private TCC framework, loaded with `dlopen` so its absence degrades to "unknown" instead of a crash; keep that fallback when changing it. Under `npm run tauri dev`, macOS attributes both permissions to your terminal app instead.
- **Migrations:** schema changes are numbered SQL files in `src-tauri/migrations/`, applied in order through `PRAGMA user_version`. Never edit a shipped migration; add a new one and register it in `src-tauri/src/store/migrations.rs`.
- **Meeting list and search:** `list_meetings` returns one page at a time, keyed on `(created_at, id)` with a cursor rather than an offset, and takes an optional search query. Search uses external-content FTS5 indexes over `meetings.title`, `notes.body_markdown`, and `transcript_segments.text` (`src-tauri/migrations/0005_meeting_search.sql`, `src-tauri/src/store/search.rs`). Triggers keep them in step, so a migration that renames or rebuilds one of those tables or columns must recreate its index and triggers too.
- **Frontend data access:** call commands only through `src/lib/api.ts`. Tests mock `@tauri-apps/api/core` with an in-memory fake backend (see `src/App.test.tsx`).
- **Settings:** stored as key/value rows in the `settings` table through `Store::setting` and `Store::set_setting` (`src-tauri/src/store/settings.rs`). The selected summary model, the chosen microphone, each source's speech sensitivity, and whether recordings keep their audio live there. A missing microphone falls back to the system default and reports a `recording:issue`.
- **Markdown storage:** notes and summaries are stored as Markdown.
- **Export format:** each file has YAML front matter (title, local date with offset, duration), then `# Title`, `## Summary`, `## My notes`, and `## Transcript`, in that order. Headings inside the summary and notes are moved down one level. Folder exports never overwrite existing files; they add a numeric suffix. Times use the system time zone through `jiff`.
- **Offline only:** the app must not make network requests at runtime. The CSP in `src-tauri/tauri.conf.json` enforces this for the UI. There are two exceptions, both started by the user. The model download in `src-tauri/src/models/download.rs` fetches only entries from the pinned registry in `src-tauri/src/models/mod.rs` (the UI passes a model ID, never a URL) and verifies size and SHA-256 before use. **Check for updates** in `src-tauri/src/updates.rs` asks GitHub's API for the latest release of `tapresle/Earshot-Notes` when you press the button, never on its own; **Open download page** opens a URL built from the release's version, never one from the response. Both run in Rust, so the webview's CSP doesn't change. The update check is switched off (`updates::ENABLED`) until the repository is public, because GitHub's API answers 404 for a private repository without signing in, and the app never carries a token. Don't add other network calls.
- **Logging:** Rust code logs with the `log` macros (`log::error!`, `log::warn!`, `log::info!`, `log::debug!`), never `eprintln!`. `tauri-plugin-log`, set up in `src-tauri/src/lib.rs`, writes to stderr and to `~/Library/Logs/com.tedpresley.earshot/Earshot Notes.log` (named after `productName`), keeping one file of at most 1 MB. Logs stay on the device: don't add network or webview targets, and don't log transcript, notes, or summary text.
- **Network test:** `cargo test --lib -- --ignored resumes_from_hugging_face` checks a real ranged download from Hugging Face. It's ignored by default because it needs the network.
- **Styling** uses the CSS custom properties in `src/styles.css`. Every color needs a dark mode value.
- **Screenshots:** `docs/screenshots/` holds a light and a dark screenshot of each scene in `src/preview/scenes.ts`, so a pull request that changes the UI shows the change in its diff. They're generated, and committed on purpose, like `third-party-notices.txt`. `preview.html` renders the real `App` with `mockIPC` answered by the fake backend in `src/testing/fakeBackend.ts`, which `App.test.tsx` also uses; `vite build` bundles only `index.html`, so neither ships. `screenshots/scenes.spec.ts` opens each scene with Playwright at the app's default window size, a fixed clock, time zone, and locale, so unchanged UI renders identical images. Macs on different macOS versions draw text edges slightly differently, so the check tolerates a few pixels of edge shading per image (`playwright.config.ts` records the measurements); it still fails on a changed word, layout, or clearly visible color, but a very subtle change, such as a slightly different shade, can pass. `npm run screenshots` rewrites only images that differ beyond that tolerance. To add a scene, add it to `scenes.ts` and a test to `scenes.spec.ts`, then run `npm run screenshots`. Scenes use synthetic data only.
- **Summary tests:** `src-tauri/tests/summarize.rs` runs a tiny real model (SmolLM2 135M, fetched by `scripts/fetch-native.sh`) to test plumbing such as streaming, limits, and cancellation, not summary quality. Logic above the model is tested through the `Generator` trait with fakes. Set `EARSHOT_TEST_CPU=1` to run that model on the CPU (`summarize::llama::Placement::Cpu`); CI does, because the hosted runner's virtual GPU makes these tests take minutes. The app always uses the GPU.
- **Rendering model output:** never render model output as HTML. Use `src/features/summary/Markdown.tsx`.
- **Test data:** tests use synthetic audio and text fixtures only. Generate speech fixtures with macOS `say`, for example `say -o tests/fixtures/name.wav --data-format=LEI16@16000 "text"`.
- **Native binaries:** `libneedle.a` and `.cact` model files aren't committed. `scripts/fetch-native.sh` pins their Hugging Face revisions and SHA-256 checksums. To change a version, update the pins and checksums there.
- **needle engine:** the engine holds one process-global, non-thread-safe model, so only one `Whistle` can exist at a time and it must stay on a single owner thread. `Whistle::transcribe` always passes English (`LANGUAGE` in `whistle.rs`): per-clip language detection read short or quiet call audio as other languages.
- **Git and pull requests:** don't add AI attribution anywhere: no `Co-Authored-By` or session trailers in commits, no "Generated with" footers or session links in pull requests, and no AI names in branch names. Commit as the repository owner. Name branches by purpose, such as `feature/<issue>-<topic>`, `bugfix/<issue>-<topic>`, or `chore/<topic>`.

## Updating pinned versions

Earshot pins exact versions, and nothing moves a pin without a pull request:

- **needle and Whistle:** revisions and checksums in `scripts/fetch-native.sh`.
- **Summary models:** repository, revision, and SHA-256 in `src-tauri/src/models/mod.rs`.
- **llama.cpp:** `llama-cpp-2` pinned with `=` in `src-tauri/Cargo.toml`, matching `RUNTIME_VERSION`.
- **Everything else:** `Cargo.lock`, `package-lock.json`, and the action SHAs.

The upstream workflow reports the first three; it counts a Hugging Face pin as outdated only when the pinned file itself changed, so README edits don't show up. Dependabot covers the rest and skips `llama-cpp-2` and `llama-cpp-sys-2`. A release takes an update when:

- **It fixes a security problem:** in the next release.
- **llama.cpp:** a model you want needs it, or it fixes a problem Earshot has. Run the summary tests and the summary benchmark (#8) on the new version, then update `RUNTIME_VERSION` and each model's `min_runtime` with the pin.
- **needle and Whistle:** it improves accuracy or speed. Run the transcription tests and listen through recordings kept with **Keep audio recordings**. Move them together: each Whistle version has to match its engine.
- **A summary model's file changed:** compare the new file with the old one in the summary benchmark (#8) before moving the pin. Earlier releases are unaffected: they download from the revision they pin and check its SHA-256.
- **Dependabot's grouped updates:** when CI passes. Read the changelogs of major version bumps first.

Every pin change gets a line in the next release notes.

## Releases

Each release has notes in `docs/releases/vX.Y.Z.md`. To make a release:

1. Set the version in `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`. They must match.
2. Run `./scripts/generate-notices.sh` to regenerate `src-tauri/resources/licenses/third-party-notices.txt`, and commit the result.
3. Write the release notes. The first release, `v0.1.0`, describes everything in the app. Later releases describe what changed since the previous one, grouped as new, improved, and fixed, and keep a known limitations section current. Use absolute links, such as `https://github.com/tapresle/Earshot-Notes/blob/main/LICENSE`, because the notes also become the GitHub release description, where relative links break.
4. Build with `scripts/build-macos.sh`, signed with Developer ID and with notarization credentials set, so it ends with "Notarized and stapled". Don't release a build that isn't notarized. The disk image is `src-tauri/target/release/bundle/dmg/Earshot Notes_X.Y.Z_aarch64.dmg`.
5. After the release branch is merged, tag the merge on `main` as `vX.Y.Z` with an annotated tag.
6. Push `main` and the tag to `origin`, then publish a GitHub release for the tag with the disk image attached. Use the release notes without their title line as the description, and add a **Download** section with the disk image's SHA-256. For example:

   ```bash
   gh release create vX.Y.Z <copy>/Earshot-Notes_X.Y.Z_aarch64.dmg --title "Earshot Notes X.Y.Z" --notes-file <notes> --verify-tag
   ```

   Upload a copy named `Earshot-Notes_X.Y.Z_aarch64.dmg`, not the build's own `Earshot Notes_X.Y.Z_aarch64.dmg`. GitHub replaces spaces in asset names with periods, and the hyphenated name keeps download links predictable.
