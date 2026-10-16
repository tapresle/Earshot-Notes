# App shell and lifecycle

[Architecture overview](README.md)

Earshot Notes runs in the menu bar. The window can close while a recording continues, and quitting has to free the native models in a set order. This page covers startup, the menu bar and window, quitting, exporting, the network boundary, and how the app is packaged.

## Startup

```mermaid
sequenceDiagram
    autonumber
    participant Tauri as tauri::Builder (lib.rs run)
    participant Store
    participant TW as TranscriptionWorker
    participant MW as ModelWorker
    participant Tray as tray
    participant Win as window

    Tauri->>Tauri: logger (stderr + log file), dialog plugin
    Tauri->>Store: open earshot.db, apply migrations
    Tauri->>Store: close_interrupted_recordings
    Tauri->>TW: spawn: load Whistle (app data copy, else bundled)
    Note over TW: If Whistle won't load, recording<br/>is unavailable, but the rest still works.
    Tauri->>Store: read transcription_model
    Tauri->>TW: preload(engine): load Whisper if chosen
    Tauri->>MW: spawn (no model loaded yet)
    Tauri->>Tauri: manage Store, Recorder, Recordings,<br/>Transcription, ModelWorker, Downloads
    Tauri->>Tray: create the menu bar icon and menu
    alt opened at login
        Tauri->>Win: hide the Dock icon, stay in the menu bar
    else opened by you
        Tauri->>Win: show the window (it starts hidden)
    end
    Win->>Win: the page asks setup_complete<br/>and shows the setup checklist if not
```

## Menu bar and window

```mermaid
stateDiagram-v2
    state "Window shown, Dock icon shown" as Shown
    state "Window hidden, menu bar only" as Hidden
    [*] --> Shown: opened by you
    [*] --> Hidden: opened at login
    Shown --> Hidden: close the window<br/>(closing is prevented and the window hides)
    Hidden --> Shown: menu bar "Open", or the Dock icon
    Shown --> [*]: Quit
    Hidden --> [*]: Quit
```

- **The page stays loaded while hidden**, so it keeps receiving events and reopens where it was.
- **The Dock icon** is shown and hidden only through `macos_audio::show_dock_icon` and `hide_dock_icon`, on the main thread. Tauri's activation policy redraws the whole menu bar.
- **Recording from the menu bar:** the toggle runs on the blocking pool. Starting creates an untitled meeting and records it, deleting the meeting again if recording can't start. Starting and stopping use the same `commands::recording::start` and `stop` as the window, which emit `recording:changed` and update the menu's status and icon. The window then opens the new meeting.
- **Recording a new meeting from the window:** with **Start recording when I create a new meeting** on (`record_new_meetings` in `settings`), **New meeting** opens the meeting and its `RecordingControls` starts recording once, as if you clicked **Start recording**, so progress and errors look the same. Unlike the menu bar, the window keeps the meeting when recording can't start, and starts nothing while another meeting is recording.

## Quitting

**Quit** in the menu bar or the app menu calls `app.exit(0)`. `RunEvent::Exit` then frees everything native in order, because ggml's Metal backend aborts the process at exit while a model is loaded.

```mermaid
sequenceDiagram
    autonumber
    participant Exit as RunEvent::Exit
    participant Rec as recording::stop_recording
    participant TW as TranscriptionWorker
    participant MW as ModelWorker

    Exit->>Rec: stop any recording<br/>(drop captures, join pipelines, set ended_at)
    Exit->>TW: shutdown (up to 3 s):<br/>skip queued jobs, free Whistle and Whisper
    Exit->>MW: shutdown (up to 3 s):<br/>cancel the running job, free the model
    Note over Exit: The process exits
```

Quitting calls `recording::stop_recording` directly rather than `commands::recording::stop`, since there's no window or menu left to update.

## Exporting

```mermaid
flowchart LR
    one["Export one meeting<br/>(MeetingView)"] --> savePath["pickSavePath<br/>(suggested: export_file_name)"] --> exportOne["export_meeting"]
    many["Export selected<br/>(sidebar)"] --> folder["pickFolder"] --> exportMany["export_meetings(ids)"]
    all["Export all<br/>(Settings)"] --> folder2["pickFolder"] --> exportAll["export_meetings(null)"]
    exportOne --> render
    exportMany --> render
    exportAll --> render
    render["export::render_meeting<br/>YAML front matter, then Title,<br/>Summary, My notes, Chat, Transcript;<br/>headings moved down a level;<br/>citations as [hh:mm:ss] or [My notes];<br/>times in the system time zone"]
    render --> write["Folder exports never overwrite:<br/>a numeric suffix is added"]
```

## The network boundary

```mermaid
flowchart TB
    subgraph webview["Webview"]
        page["React app"]
        csp["CSP: default-src 'self';<br/>connect-src ipc: only"]
    end
    subgraph rust["Rust core (ureq)"]
        dl["models::download<br/>only pinned registry URLs,<br/>started by you"]
        upd["updates::check<br/>GitHub latest release,<br/>when you press the button;<br/>refused while updates::ENABLED is false"]
    end
    open["Opened in your browser,<br/>not fetched by the app:<br/>a model's Hugging Face page,<br/>a release page built from the version"]
    page -. "blocked" .-> internet(["Internet"])
    dl --> internet
    upd --> internet
    page --> open
```

Logs stay on the Mac, and never include transcript, notes, or summary text.

## Packaging

```mermaid
flowchart LR
    subgraph sources["Inputs"]
        fetch["scripts/fetch-native.sh<br/>pinned libneedle.a, whistle.cact,<br/>test models"]
        buildW["scripts/build-whisper.sh<br/>whisper.cpp at a pinned tag,<br/>own ggml, only earshot_whisper_*<br/>exported"]
        cargo["cargo: Rust core,<br/>llama.cpp (Metal) via llama-cpp-2,<br/>macos-audio (Objective-C)"]
        vite["vite build: index.html only"]
        notices["scripts/generate-notices.sh"]
    end
    subgraph app["Earshot Notes.app"]
        bin["Contents/MacOS: the app binary<br/>Rust core + needle (static)<br/>+ llama.cpp"]
        fw["Frameworks/libearshot_whisper.dylib<br/>opened with dlopen on first use"]
        res["Resources/models/whistle.cact<br/>Resources/licenses/"]
        web["bundled web assets"]
    end
    fetch --> bin
    fetch --> res
    buildW --> fw
    cargo --> bin
    vite --> web
    notices --> res
    app --> sign["scripts/build-macos.sh<br/>sign (hardened runtime), notarize,<br/>staple, DMG, Gatekeeper check"]
```

whisper.cpp and llama.cpp each include ggml, and two copies can't be linked into one binary. That's why whisper.cpp is a separate dynamic library that exports only Earshot's C interface.

## Where the code is

| Concern | Code |
|---|---|
| Startup, commands list, quitting | `src-tauri/src/lib.rs` |
| Menu bar | `src-tauri/src/tray.rs` |
| Window and Dock icon | `src-tauri/src/window.rs`, `src-tauri/crates/macos-audio` |
| Export | `src-tauri/src/export.rs`, `src-tauri/src/commands/export.rs`, `src-tauri/src/store/export.rs` |
| Update check | `src-tauri/src/updates.rs`, `src-tauri/src/commands/updates.rs` |
| Content Security Policy, bundle | `src-tauri/tauri.conf.json` |
| Build and release | `scripts/build-macos.sh`, `docs/building.md` |
