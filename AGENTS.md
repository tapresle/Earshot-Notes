# Earshot

A local-first meeting notes app. It transcribes your microphone and system audio on the device with Cactus Whistle, gives you a notepad during the meeting, and on request summarizes your notes and the transcript with a local LLM. It makes no network calls at runtime.

macOS (Apple Silicon) is the first target. Windows and Linux follow.

## Stack

- **Shell:** Tauri 2. The Rust core lives in `src-tauri/`.
- **UI:** React 19, TypeScript, and Vite in `src/`.
- **Transcription:** Cactus Whistle through the needle C API (`needle.h`). This is planned, not built yet.
- **Summaries:** llama.cpp embedded through `llama-cpp-2`. This is planned, not built yet.
- **Storage:** SQLite through `rusqlite`. This is planned, not built yet.

The decision records in `docs/decisions/` explain why each piece was chosen.

## Commands

Run all commands from the repository root unless noted.

| Task | Command |
| --- | --- |
| Install JS dependencies | `npm install` |
| Run the app in development | `npm run tauri dev` |
| Build the app bundle | `npm run tauri build` |
| Frontend lint | `npm run lint` |
| Frontend typecheck | `npm run typecheck` |
| Frontend tests | `npm test` |
| Rust lint | `cargo clippy --all-targets -- -D warnings` (in `src-tauri/`) |
| Rust format check | `cargo fmt --check` (in `src-tauri/`) |
| Rust tests | `cargo test` (in `src-tauri/`) |

Before you hand off work, run the lint, typecheck, test, and format commands. All of them must pass.

## Conventions

- **UI features** live under `src/features/<feature>/`. Each feature keeps its components, hooks, and tests together.
- **Rust modules** under `src-tauri/src/` follow the plan's boundaries: `audio`, `chunker`, `transcribe`, `summarize`, `store`, `models`, `export`, and `commands`. Tauri command handlers in `commands` stay thin and call into the other modules.
- **UI and core communication:** the UI calls Tauri commands for requests, and the core pushes live updates as Tauri events (`transcript:segment`, `summary:token`, `summary:done`, `recording:state`).
- **Markdown storage:** notes and summaries are stored as Markdown.
- **Offline only:** the app must not make network requests at runtime. The CSP in `src-tauri/tauri.conf.json` enforces this for the UI. The only planned exception is an explicit, user-started model download.
- **Styling** uses the CSS custom properties in `src/styles.css`. Every color needs a dark mode value.
- **Test data:** tests use synthetic audio and text fixtures only.
