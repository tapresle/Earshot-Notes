# Earshot

A local-first meeting notes app. It transcribes your microphone and system audio on the device with Cactus Whistle, gives you a notepad during the meeting, and on request summarizes your notes and the transcript with a local LLM. It makes no network calls at runtime.

macOS (Apple Silicon) is the first target. Windows and Linux follow.

## Stack

- **Shell:** Tauri 2. The Rust core lives in `src-tauri/`.
- **UI:** React 19, TypeScript, and Vite in `src/`.
- **Transcription:** Cactus Whistle through the prebuilt needle engine. `src-tauri/crates/needle-sys` holds the raw FFI, and `src-tauri/src/transcribe/whistle.rs` is the safe wrapper.
- **Summaries:** llama.cpp embedded through `llama-cpp-2`. This is planned, not built yet.
- **Storage:** SQLite through `rusqlite`, in `src-tauri/src/store/`. The database lives in the app data directory as `earshot.db`.

The decision records in `docs/decisions/` explain why each piece was chosen.

## Commands

Run all commands from the repository root unless noted.

| Task | Command |
| --- | --- |
| Fetch the native engine and models (after cloning, or when a pin changes) | `./scripts/fetch-native.sh` |
| Install JS dependencies | `npm install` |
| Run the app in development | `npm run tauri dev` |
| Build the app bundle | `npm run tauri build` |
| Frontend lint | `npm run lint` |
| Frontend typecheck | `npm run typecheck` |
| Frontend tests | `npm test` |
| Rust lint | `cargo clippy --workspace --all-targets -- -D warnings` (in `src-tauri/`) |
| Rust format check | `cargo fmt --check` (in `src-tauri/`) |
| Rust tests | `cargo test --workspace` (in `src-tauri/`) |

Before you hand off work, run the lint, typecheck, test, and format commands. All of them must pass.

## Conventions

- **UI features** live under `src/features/<feature>/`. Each feature keeps its components, hooks, and tests together.
- **Rust modules** under `src-tauri/src/`: `audio` (capture, downmix, resampling), `chunker` (speech segmentation), `transcribe` (Whistle and its worker thread), `recording` (start, stop, and the capture-to-worker pipeline), `store`, and `commands`. Planned: `summarize`, `models`, and `export`. Tauri command handlers in `commands` stay thin and call into the other modules.
- **UI and core communication:** the UI calls Tauri commands for requests, and the core pushes live updates as Tauri events. Current events: `transcript:segment`. Planned: `summary:token` and `summary:done`.
- **Audio pipeline:** capture threads must never block. They drop buffers when the pipeline falls behind. Whistle receives 16 kHz mono segments of at most 28 s from `chunker`. Transcript times are offsets from `meetings.started_at`.
- **Microphone permission:** the bundled app declares `NSMicrophoneUsageDescription` in `src-tauri/Info.plist`. Under `npm run tauri dev`, macOS attributes the microphone to your terminal app instead.
- **Migrations:** schema changes are numbered SQL files in `src-tauri/migrations/`, applied in order through `PRAGMA user_version`. Never edit a shipped migration; add a new one and register it in `src-tauri/src/store/migrations.rs`.
- **Frontend data access:** call commands only through `src/lib/api.ts`. Tests mock `@tauri-apps/api/core` with an in-memory fake backend (see `src/App.test.tsx`).
- **Markdown storage:** notes and summaries are stored as Markdown.
- **Offline only:** the app must not make network requests at runtime. The CSP in `src-tauri/tauri.conf.json` enforces this for the UI. The only planned exception is an explicit, user-started model download.
- **Styling** uses the CSS custom properties in `src/styles.css`. Every color needs a dark mode value.
- **Test data:** tests use synthetic audio and text fixtures only. Generate speech fixtures with macOS `say`, for example `say -o tests/fixtures/name.wav --data-format=LEI16@16000 "text"`.
- **Native binaries:** `libneedle.a` and `.cact` model files aren't committed. `scripts/fetch-native.sh` pins their Hugging Face revisions and SHA-256 checksums. To change a version, update the pins and checksums there.
- **needle engine:** the engine holds one process-global, non-thread-safe model, so only one `Whistle` can exist at a time and it must stay on a single owner thread.
