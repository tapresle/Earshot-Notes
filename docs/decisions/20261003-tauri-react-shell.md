# Tauri 2 with a React UI as the app shell

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

The app ships on macOS first and needs a path to Windows and Linux. The core work is native: audio capture, Whistle transcription through a C API, and llama.cpp inference. The UI is a document-style editor with lists and panels.

## Decision

Use Tauri 2. The Rust core owns audio, inference, and storage, and a React and TypeScript UI runs in the system webview.

## Alternatives considered

- **Native SwiftUI:** This has the best macOS feel and the simplest system audio capture. Windows and Linux would need a full UI rewrite.
- **Electron:** This has mature cross-platform tooling. It has a much larger footprint (about 150 MB), and native inference would have to go through Node addons.

## Consequences

- Rust calls the C APIs of Whistle and llama.cpp directly, with no extra runtime in between.
- Most of the code is shared across platforms. Platform-specific work is limited to system audio capture and native library builds.
- Webview rendering differs slightly between platforms (WebKit, WebView2, and WebKitGTK). UI tests run in jsdom, so you check platform differences manually.
