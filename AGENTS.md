# Earshot

A local-first meeting notes app. It transcribes your microphone and system audio on the device with Cactus Whistle, gives you a notepad during the meeting, and on request summarizes your notes and the transcript with a local LLM. It makes no network calls at runtime.

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
| Frontend lint | `npm run lint` |
| Frontend typecheck | `npm run typecheck` |
| Frontend tests | `npm test` |
| Rust lint | `cargo clippy --workspace --all-targets -- -D warnings` (in `src-tauri/`) |
| Rust format check | `cargo fmt --check` (in `src-tauri/`) |
| Rust tests | `cargo test --workspace` (in `src-tauri/`) |

Before you hand off work, run the lint, typecheck, test, and format commands. All of them must pass.

## Conventions

- **UI features** live under `src/features/<feature>/`. Each feature keeps its components, hooks, and tests together.
- **Rust modules** under `src-tauri/src/`: `audio` (microphone and system capture, downmix, resampling), `chunker` (speech segmentation), `transcribe` (Whistle and its worker thread), `recording` (start, stop, and the capture-to-worker pipeline), `summarize` (prompting, the llama.cpp generator, and the background summarizer), `models` (known models, download, and GGUF import), `export` (Markdown rendering and file writing), `store`, and `commands`. Tauri command handlers in `commands` stay thin and call into the other modules. Synchronous commands run on the main thread, so a command that touches the database, the disk, or another thread is `async` and does that work through `commands::blocking`, which moves it to the blocking thread pool. `Store` serializes access to its one connection with a mutex, and shared state such as `Store`, `Recorder`, and `Summarizer` is managed as an `Arc` so the work can take its own handle.
- **UI and core communication:** the UI calls Tauri commands for requests, and the core pushes live updates as Tauri events. Current events: `transcript:segment`, `recording:issue`, `summary:progress`, `summary:text`, `summary:done`, `summary:failed`, `model:download-progress`, `model:download-done`, and `model:download-failed`.
- **Audio pipeline:** capture threads must never block. They drop buffers when the pipeline falls behind. Whistle receives 16 kHz mono segments of at most 28 s from `chunker`. Transcript times are offsets from `meetings.started_at`.
- **Packaging:** `scripts/build-macos.sh` signs with `APPLE_SIGNING_IDENTITY`, else the first Developer ID or Apple Development certificate in the keychain, else ad-hoc. The build uses the hardened runtime, so `src-tauri/Entitlements.plist` must keep `com.apple.security.device.audio-input`, or microphone input is silently denied. The deployment target is macOS 26 for every build, set in `src-tauri/.cargo/config.toml`; keep it equal to `minimumSystemVersion` in `tauri.conf.json`. After changing it, run `cargo clean` in `src-tauri/`: llama.cpp's build script doesn't notice the change, and a stale build fails to link with an undefined `__isPlatformVersionAtLeast`. Bundled license texts live in `src-tauri/resources/licenses/`.
- **Native macOS code:** `src-tauri/crates/macos-audio` holds the Objective-C system audio tap and microphone permission calls, compiled by its `build.rs` with `cc` under `-Wall -Wextra -Werror`. Keep its C interface in `native/audio_tap.h` and its Rust bindings in `src/lib.rs` in sync.
- **Permissions:** the bundled app declares `NSMicrophoneUsageDescription` and `NSAudioCaptureUsageDescription` in `src-tauri/Info.plist`, and requires macOS 26, the oldest version the prebuilt needle engine supports. System audio permission is read and requested through the private TCC framework, loaded with `dlopen` so its absence degrades to "unknown" instead of a crash; keep that fallback when changing it. Under `npm run tauri dev`, macOS attributes both permissions to your terminal app instead.
- **Migrations:** schema changes are numbered SQL files in `src-tauri/migrations/`, applied in order through `PRAGMA user_version`. Never edit a shipped migration; add a new one and register it in `src-tauri/src/store/migrations.rs`.
- **Frontend data access:** call commands only through `src/lib/api.ts`. Tests mock `@tauri-apps/api/core` with an in-memory fake backend (see `src/App.test.tsx`).
- **Settings:** stored as key/value rows in the `settings` table through `Store::setting` and `Store::set_setting` (`src-tauri/src/store/settings.rs`). The selected summary model and the chosen microphone live there. A missing microphone falls back to the system default and reports a `recording:issue`.
- **Markdown storage:** notes and summaries are stored as Markdown.
- **Export format:** each file has YAML front matter (title, local date with offset, duration), then `# Title`, `## Summary`, `## My notes`, and `## Transcript`, in that order. Headings inside the summary and notes are moved down one level. Folder exports never overwrite existing files; they add a numeric suffix. Times use the system time zone through `jiff`.
- **Offline only:** the app must not make network requests at runtime. The CSP in `src-tauri/tauri.conf.json` enforces this for the UI. The one exception is the user-started model download in `src-tauri/src/models/download.rs`. It fetches only entries from the pinned registry in `src-tauri/src/models/mod.rs` (the UI passes a model ID, never a URL) and verifies size and SHA-256 before use. Don't add other network calls.
- **Logging:** Rust code logs with the `log` macros (`log::error!`, `log::warn!`, `log::info!`, `log::debug!`), never `eprintln!`. `tauri-plugin-log`, set up in `src-tauri/src/lib.rs`, writes to stderr and to `~/Library/Logs/com.tedpresley.earshot/Earshot.log`, keeping one file of at most 1 MB. Logs stay on the device: don't add network or webview targets, and don't log transcript, notes, or summary text.
- **Network test:** `cargo test --lib -- --ignored resumes_from_hugging_face` checks a real ranged download from Hugging Face. It's ignored by default because it needs the network.
- **Styling** uses the CSS custom properties in `src/styles.css`. Every color needs a dark mode value.
- **Summary tests:** `src-tauri/tests/summarize.rs` runs a tiny real model (SmolLM2 135M, fetched by `scripts/fetch-native.sh`) to test plumbing such as streaming, limits, and cancellation, not summary quality. Logic above the model is tested through the `Generator` trait with fakes.
- **Rendering model output:** never render model output as HTML. Use `src/features/summary/Markdown.tsx`.
- **Test data:** tests use synthetic audio and text fixtures only. Generate speech fixtures with macOS `say`, for example `say -o tests/fixtures/name.wav --data-format=LEI16@16000 "text"`.
- **Native binaries:** `libneedle.a` and `.cact` model files aren't committed. `scripts/fetch-native.sh` pins their Hugging Face revisions and SHA-256 checksums. To change a version, update the pins and checksums there.
- **needle engine:** the engine holds one process-global, non-thread-safe model, so only one `Whistle` can exist at a time and it must stay on a single owner thread.
- **Git and pull requests:** don't add AI attribution anywhere: no `Co-Authored-By` or session trailers in commits, no "Generated with" footers or session links in pull requests, and no AI names in branch names. Commit as the repository owner. Name branches by purpose, such as `feature/<issue>-<topic>`, `bugfix/<issue>-<topic>`, or `chore/<topic>`.

## Releases

Each release has notes in `docs/releases/vX.Y.Z.md`. To make a release:

1. Set the version in `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`. They must match.
2. Write the release notes. The first release, `v0.1.0`, describes everything in the app. Later releases describe what changed since the previous one, grouped as new, improved, and fixed, and keep a known limitations section current. Use absolute links, such as `https://github.com/tapresle/Earshot/blob/main/LICENSE`, because the notes also become the GitHub release description, where relative links break.
3. Build with `scripts/build-macos.sh`. The disk image is `src-tauri/target/release/bundle/dmg/Earshot_X.Y.Z_aarch64.dmg`.
4. After the release branch is merged, tag the merge on `main` as `vX.Y.Z` with an annotated tag.
5. Push `main` and the tag to `origin`, then publish a GitHub release for the tag with the disk image attached. Use the release notes without their title line as the description, and add a **Download** section with the disk image's SHA-256. For example:

   ```bash
   gh release create vX.Y.Z src-tauri/target/release/bundle/dmg/Earshot_X.Y.Z_aarch64.dmg --title "Earshot X.Y.Z" --notes-file <notes> --verify-tag
   ```
