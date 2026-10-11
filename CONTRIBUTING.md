# Contributing to Earshot Notes

Thanks for your interest in Earshot Notes. Bug reports, ideas, and pull requests are all welcome.

## Never share real meeting content

Earshot Notes handles private conversations, so the project has one rule above the others: never put real meeting content in an issue, a pull request, a commit, or a test. That includes transcripts, summaries, notes, recordings, screenshots of real meetings, and the names of people, customers, or products from them.

When a bug depends on what was said, describe its shape instead, such as "a two-hour call where the participants' side drops out after an hour", or reproduce it with invented text or synthetic speech from `say`.

## Report a bug or suggest a feature

Open an issue from one of the templates. For a bug, include the Earshot Notes version you installed, your macOS version, your Mac's chip and memory, which transcription model and local AI model you use, and the steps that cause it.

To report a security problem, follow [`SECURITY.md`](SECURITY.md) instead of opening an issue.

## Set up

Earshot Notes builds on a Mac with Apple Silicon and macOS 26 or later. [Building Earshot Notes](docs/building.md) lists the tools to install and covers signing.

1. Fetch the native engine and models, and build whisper.cpp:

   ```bash
   ./scripts/fetch-native.sh
   ```

2. Install the JavaScript dependencies:

   ```bash
   npm install
   ```

3. Run the app in development:

   ```bash
   npm run tauri dev
   ```

Under `npm run tauri dev`, macOS asks your terminal app for the microphone and system audio permissions, not Earshot Notes.

## Make a change

1. Start from an issue, so the change has agreed goals. For anything larger than a small fix, comment on the issue with your approach before you build it.
2. Branch from `main`, named by purpose: `feature/<issue>-<topic>`, `bugfix/<issue>-<topic>`, or `chore/<topic>`.
3. Start each commit subject with the issue number and an imperative summary, such as `#42: Allow sorting by created date`. Use the body to explain why.
4. Add tests for what you change. For a bug, start with a test that fails.
5. Run the checks below, then open a pull request that fills in the template and says `Closes #<issue>`.

## Run the checks

Every check must pass before a pull request is reviewed. CI runs them too.

| Check | Command |
| --- | --- |
| Frontend lint | `npm run lint` |
| Frontend typecheck | `npm run typecheck` |
| Frontend tests | `npm test` |
| Rust format (in `src-tauri/`) | `cargo fmt --check` |
| Rust lint (in `src-tauri/`) | `cargo clippy --workspace --all-targets -- -D warnings` |
| Rust tests (in `src-tauri/`) | `cargo test --workspace` |
| UI screenshots (needs `npx playwright install chromium` once) | `npm run screenshots:check` |

When you change the UI on purpose, run `npm run screenshots` and commit the updated images with the change.

## Conventions

[`AGENTS.md`](AGENTS.md) describes the architecture, the conventions, and the rules each part of the app follows, and [`docs/architecture/`](docs/architecture/README.md) has diagrams of each subsystem. A few rules matter for every change:

- **Offline only:** the app makes no network requests except the model downloads and the update check that the user starts. Don't add others.
- **Synthetic test data:** tests, fixtures, screenshots, and docs use invented names and text, and synthetic speech.
- **Pinned versions:** models, native engines, and llama.cpp are pinned by revision and checksum. Changing a pin needs measurements, as **Updating pinned versions** in `AGENTS.md` describes.
- **No AI attribution:** no `Co-Authored-By` trailers for AI tools, and no "Generated with" footers.

## License

Earshot Notes is licensed under the [GNU General Public License, version 3](LICENSE) only. By contributing, you agree that your contribution is licensed the same way.
